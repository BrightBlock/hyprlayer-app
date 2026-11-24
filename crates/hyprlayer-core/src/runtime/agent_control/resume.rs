//! PTY agents a server stop cut off, resumed in place: the same terminal id, workstream, title,
//! spawner and launch settings, on `--resume` of their own conversation.

use std::sync::{Arc, Weak};
use std::time::Duration;

use hyprlayer_models::{
    PermissionMode, PtyAgent, PtyAgentState, PtyLaunchSession, RuntimeEvent, UiRequest,
    WorkstreamStatus,
};
use tokio::time::Instant;

use super::links::AgentLinks;
use super::spawn::{SPAWN_COLS, SPAWN_ROWS};
use super::{blocking, denied, AgentControlService};
use crate::error::{CoreError, Result};
use crate::runtime::goals::OVERLAY_WAIT;
use crate::runtime::terminals::{LaunchAgentRequest, PtyLineage, SessionExistsRequest};
use crate::runtime::HyprlayerRuntime;
use crate::store::{Db, PtyAgentRow, PtyAgentSync};
use crate::terminal::{text, PtySession};

/// Restart resumes an agent gets without finishing a turn before it stays exited.
pub(crate) const MAX_PTY_RESUMES: u32 = 3;
const STARTED_WAIT: Duration = Duration::from_secs(60);
const STARTED_POLL: Duration = Duration::from_millis(250);
const EXIT_LINES: usize = 6;

/// Why a stored agent stays exited after a restart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Skip {
    /// It wasn't mid-turn: the tab's Restart, or a message from its spawner, resumes it.
    Idle,
    Archived,
    NoConversation,
    Exhausted(u32),
    TooDeep {
        depth: u32,
        max: u32,
    },
}

impl Skip {
    fn reason(&self) -> String {
        match self {
            Skip::Idle => "it was idle".into(),
            Skip::Archived => "its workstream is archived".into(),
            Skip::NoConversation => "it never started a conversation to resume".into(),
            Skip::Exhausted(n) => format!(
                "the last {n} restart resumes never finished a turn, so it may be what stops the server"
            ),
            Skip::TooDeep { depth, max } => {
                format!("it is {depth} agents deep, past the Max depth of {max}")
            }
        }
    }
}

/// Whether a restart resumes a stored agent: only one cut off mid-turn, in a live workstream,
/// with a conversation, under the depth limit and the resume bound.
pub(crate) fn restart_skip(row: &PtyAgentRow, archived: bool, max_depth: u32) -> Option<Skip> {
    if archived {
        return Some(Skip::Archived);
    }
    if !row.working {
        return Some(Skip::Idle);
    }
    if row.provider_session_id.is_none() {
        return Some(Skip::NoConversation);
    }
    if row.resumes >= MAX_PTY_RESUMES {
        return Some(Skip::Exhausted(row.resumes));
    }
    if row.spawned_by.is_some() && row.depth > max_depth {
        return Some(Skip::TooDeep {
            depth: row.depth,
            max: max_depth,
        });
    }
    None
}

pub(crate) fn nudge_text() -> String {
    "[from hyprlayer]\n\nThe hyprlayer server restarted and stopped this agent mid-turn. \
     Carry on where you left off."
        .into()
}

/// The spawn lineage of agents a stopped server left, so their spawners still own them.
pub(crate) fn restore_links(db: &Db, links: &AgentLinks) {
    match db.pty_agents() {
        Ok(rows) => {
            for row in rows {
                if let Some(parent) = row.spawned_by {
                    links.link(&row.terminal_id, parent, row.depth);
                }
            }
        }
        Err(e) => tracing::warn!("the PTY agents a previous server left are unreadable: {e}"),
    }
}

pub(crate) enum ResumeCause {
    Restart,
    /// A spawner's message, delivered as the resumed agent's first turn.
    Message(String),
}

pub(crate) enum Resumed {
    Launched,
    /// Already running: something else resumed it first.
    Live,
    Skipped(Skip),
}

impl AgentControlService<'_> {
    /// Resumes a stored agent that isn't running, once: the terminal's resume lock makes a second
    /// caller find it live, and a closed tab's row is gone.
    pub(crate) async fn resume_pty(
        &self,
        terminal_id: &str,
        cause: ResumeCause,
    ) -> Result<Resumed> {
        let _resume = self.runtime.pty_resume_locks.acquire(terminal_id).await;
        if self.runtime.pty_agents.snapshot(terminal_id).is_some() {
            return Ok(Resumed::Live);
        }
        let row = self.stored(terminal_id).await?.ok_or_else(|| {
            denied("that agent's tab was closed or it ended, so there is nothing to resume")
        })?;
        let workstream = self.runtime.workstreams().find(&row.workstream_id).await?;
        let archived = workstream.status == WorkstreamStatus::Archived;
        let max_depth = self.runtime.settings.get().await.agent_control.max_depth;
        let restart = matches!(cause, ResumeCause::Restart);
        let skip = match restart {
            true => restart_skip(&row, archived, max_depth),
            false => restart_skip(
                &PtyAgentRow {
                    working: true,
                    resumes: 0,
                    ..row.clone()
                },
                archived,
                max_depth,
            ),
        };
        if let Some(skip) = skip {
            if skip == Skip::Archived {
                self.runtime.pty_agents.forget(terminal_id);
            }
            return match restart {
                true => Ok(Resumed::Skipped(skip)),
                false => Err(denied(&format!(
                    "that agent cannot resume: {}",
                    skip.reason()
                ))),
            };
        }
        let sid = row.provider_session_id.clone().unwrap_or_default();
        let _slot = match &row.spawned_by {
            Some(parent) => Some(
                self.admit_child(parent, "the agent that spawned it")
                    .await?,
            ),
            None => None,
        };
        let on_disk = self
            .runtime
            .terminals()
            .session_exists(SessionExistsRequest {
                workstream_id: row.workstream_id.clone(),
                engine_id: row.engine_id.clone(),
                session_id: sid.clone(),
            })
            .await?;
        if !on_disk {
            self.runtime.pty_agents.forget(terminal_id);
            return Err(denied(&format!(
                "its conversation {sid} is no longer on disk"
            )));
        }
        let (db, id) = (self.runtime.db.clone(), terminal_id.to_string());
        blocking(move || db.record_pty_resume(&id)).await?;
        let (cols, rows) = self
            .runtime
            .terminals
            .agent_fit()
            .unwrap_or((SPAWN_COLS, SPAWN_ROWS));
        let lineage = row.spawned_by.clone().map(|parent| PtyLineage {
            parent,
            depth: row.depth,
            mode: row.mode.unwrap_or(match row.launch.bypass {
                true => PermissionMode::BypassPermissions,
                false => PermissionMode::Default,
            }),
        });
        let (_, launched) = self
            .runtime
            .terminals()
            .launch_agent(LaunchAgentRequest {
                workstream_id: row.workstream_id.clone(),
                engine_id: row.engine_id.clone(),
                session: PtyLaunchSession {
                    session_id: Some(sid),
                    resume: true,
                    ..row.launch.clone()
                },
                cols,
                rows,
                lineage,
                resume_of: Some(PtyAgentRow {
                    resumes: row.resumes + 1,
                    ..row.clone()
                }),
            })
            .await?;
        tracing::info!(
            "terminal {terminal_id}: resumed {} in place ({} of {MAX_PTY_RESUMES})",
            row.engine_id,
            row.resumes + 1
        );
        let agents = self.runtime.pty_agents.clone();
        if let Some(title) = row.title.clone() {
            agents.set_title(terminal_id, title);
        }
        if row.spawned_by.is_some() {
            let _ = self.runtime.events.send(RuntimeEvent::UiRequested {
                request: UiRequest::OpenTerminalTab {
                    workstream_id: row.workstream_id.clone(),
                    terminal_id: terminal_id.to_string(),
                    engine_id: row.engine_id.clone(),
                    provider_session_id: launched.provider_session_id.clone(),
                    bypass: row.launch.bypass,
                },
            });
        }
        let Some(snapshot) = agents.snapshot(terminal_id) else {
            return Err(denied("it exited as soon as it was resumed"));
        };
        let text = match cause {
            ResumeCause::Restart => nudge_text(),
            ResumeCause::Message(text) => text,
        };
        super::spawn::send_first_prompt(agents, terminal_id, text);
        watch_start(self.runtime.handle.clone(), row, snapshot.session);
        Ok(Resumed::Launched)
    }

    /// A stored agent's row, when it isn't running.
    pub(crate) async fn stored(&self, terminal_id: &str) -> Result<Option<PtyAgentRow>> {
        if self.runtime.pty_agents.snapshot(terminal_id).is_some() {
            return Ok(None);
        }
        let (agents, db, id) = (
            self.runtime.pty_agents.clone(),
            self.runtime.db.clone(),
            terminal_id.to_string(),
        );
        blocking(move || {
            agents.flush_store();
            db.pty_agent(&id)
        })
        .await
    }

    /// Stored agents that aren't running, as the list verbs show one: exited.
    pub async fn list_stopped_pty_agents(&self) -> Result<Vec<PtyAgent>> {
        let (agents, db) = (self.runtime.pty_agents.clone(), self.runtime.db.clone());
        let rows = blocking(move || {
            agents.flush_store();
            db.pty_agents()
        })
        .await?;
        Ok(rows
            .into_iter()
            .filter(|row| self.runtime.pty_agents.snapshot(&row.terminal_id).is_none())
            .map(|row| PtyAgent {
                terminal_id: row.terminal_id,
                workstream_id: row.workstream_id,
                engine_id: row.engine_id,
                state: PtyAgentState::Exited,
                provider_session_id: row.provider_session_id,
                activity: None,
                last_reply: None,
                spawned_by: row.spawned_by,
                hooked: true,
                title: row.title,
                status: Default::default(),
            })
            .collect())
    }

    /// Leaves it exited, says why to its spawner and the desktops, and stops retrying it.
    async fn not_resumed(&self, row: &PtyAgentRow, reason: &str) {
        report_not_resumed(self.runtime, row, reason).await;
        let (db, id) = (self.runtime.db.clone(), row.terminal_id.clone());
        let settled = blocking(move || {
            db.sync_pty_agent(
                &id,
                &PtyAgentSync {
                    working: false,
                    ..Default::default()
                },
            )
        })
        .await;
        if let Err(e) = settled {
            tracing::warn!("terminal {}: may be retried: {e}", row.terminal_id);
        }
    }
}

async fn report_not_resumed(rt: &HyprlayerRuntime, row: &PtyAgentRow, reason: &str) {
    let name = row.title.as_deref().unwrap_or(&row.engine_id);
    tracing::warn!(
        "terminal {}: not resumed after the server restart: {reason}",
        row.terminal_id
    );
    let _ = rt.events.send(RuntimeEvent::UiRequested {
        request: UiRequest::Notify {
            workstream_id: row.workstream_id.clone(),
            title: format!("{name} was not resumed"),
            body: format!("It stays exited: {reason}."),
            session_id: None,
            terminal_id: Some(row.terminal_id.clone()),
            from_lead: false,
        },
    });
    let Some(parent) = row.spawned_by.clone() else {
        return;
    };
    let project = match rt.workstreams().find(&row.workstream_id).await {
        Ok(ws) => ws.project_path,
        Err(_) => return,
    };
    rt.agent_control().report_to(
        parent,
        project,
        format!(
            "terminal {} ({name}) was not resumed after the hyprlayer server restart: {reason}",
            row.terminal_id
        ),
    );
}

/// A resume that dies as it starts (a conversation claude can't load) says so, with its last lines.
fn watch_start(runtime: Weak<HyprlayerRuntime>, row: PtyAgentRow, session: Arc<PtySession>) {
    tokio::spawn(async move {
        let deadline = Instant::now() + STARTED_WAIT;
        loop {
            let Some(rt) = runtime.upgrade() else {
                return;
            };
            match rt.pty_agents.snapshot(&row.terminal_id) {
                Some(now) if now.agent.state != PtyAgentState::Starting => return,
                Some(_) if Instant::now() > deadline => return,
                Some(_) => {}
                None if rt.coordinator.is_stopped() => return,
                None => {
                    let cols = session.size().1.into();
                    let tail = text::tail(&session.scrollback_bytes(), cols, EXIT_LINES);
                    let tail = tail.trim();
                    let reason = match tail.is_empty() {
                        true => "it exited as soon as it was resumed".to_string(),
                        false => format!("it exited as soon as it was resumed, printing: {tail}"),
                    };
                    report_not_resumed(&rt, &row, &reason).await;
                    return;
                }
            }
            drop(rt);
            tokio::time::sleep(STARTED_POLL).await;
        }
    });
}

/// After a restart: every stored agent cut off mid-turn resumes, once a client's environment is
/// back so its binaries resolve as they did; the rest stay exited.
pub(crate) async fn resume_after_restart(runtime: &Weak<HyprlayerRuntime>) {
    let Some(rt) = runtime.upgrade() else {
        return;
    };
    let db = rt.db.clone();
    let rows = match blocking(move || db.pty_agents()).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!("could not read the PTY agents the last server stopped: {e}");
            return;
        }
    };
    if !rows.iter().any(|row| row.working) {
        return;
    }
    let mut arrived = rt.overlay_arrived.subscribe();
    drop(rt);
    if tokio::time::timeout(OVERLAY_WAIT, arrived.wait_for(|arrived| *arrived))
        .await
        .is_err()
    {
        tracing::info!(
            "no client environment within {OVERLAY_WAIT:?}; resuming PTY agents without it"
        );
    }
    let Some(rt) = runtime.upgrade() else {
        return;
    };
    let control = rt.agent_control();
    let max_depth = rt.settings.get().await.agent_control.max_depth;
    let (mut due, mut resumed) = (0, 0);
    for row in rows {
        let archived = match rt.workstreams().find(&row.workstream_id).await {
            Ok(ws) => ws.status == WorkstreamStatus::Archived,
            Err(CoreError::WorkstreamNotFound { .. }) => true,
            Err(e) => {
                tracing::warn!(
                    "terminal {}: not checked for a resume: {e}",
                    row.terminal_id
                );
                continue;
            }
        };
        let skip = restart_skip(&row, archived, max_depth);
        let outcome = match skip {
            Some(skip) => Ok(Resumed::Skipped(skip)),
            None => {
                due += 1;
                control
                    .resume_pty(&row.terminal_id, ResumeCause::Restart)
                    .await
            }
        };
        match outcome {
            Ok(Resumed::Launched) => resumed += 1,
            Ok(Resumed::Live) | Ok(Resumed::Skipped(Skip::Idle)) => {}
            Ok(Resumed::Skipped(Skip::Archived)) => rt.pty_agents.forget(&row.terminal_id),
            Ok(Resumed::Skipped(skip)) => control.not_resumed(&row, &skip.reason()).await,
            // The try is counted, so a resume that keeps failing runs out like one that keeps dying.
            Err(e) => report_not_resumed(&rt, &row, &format!("its resume failed: {e}")).await,
        }
    }
    tracing::info!("resumed {resumed} of {due} PTY agent(s) the last server stopped mid-turn");
}

#[cfg(test)]
mod tests {
    use hyprlayer_models::AgentCaller;

    use super::*;

    fn row(working: bool) -> PtyAgentRow {
        PtyAgentRow {
            terminal_id: "t1".into(),
            workstream_id: "ws".into(),
            engine_id: "claude-code".into(),
            launch: PtyLaunchSession::default(),
            spawned_by: Some(AgentCaller::Session {
                session_id: "lead".into(),
            }),
            depth: 1,
            mode: None,
            provider_session_id: Some("p1".into()),
            title: None,
            working,
            resumes: 0,
        }
    }

    #[test]
    fn only_an_agent_cut_off_mid_turn_resumes() {
        assert_eq!(restart_skip(&row(true), false, 3), None);
        assert_eq!(restart_skip(&row(false), false, 3), Some(Skip::Idle));
    }

    #[test]
    fn an_archived_workstream_wins_over_everything() {
        assert_eq!(restart_skip(&row(true), true, 3), Some(Skip::Archived));
        assert_eq!(restart_skip(&row(false), true, 3), Some(Skip::Archived));
    }

    #[test]
    fn resumes_stop_at_the_bound() {
        let mut tried = row(true);
        tried.resumes = MAX_PTY_RESUMES - 1;
        assert_eq!(restart_skip(&tried, false, 3), None);
        tried.resumes = MAX_PTY_RESUMES;
        assert_eq!(
            restart_skip(&tried, false, 3),
            Some(Skip::Exhausted(MAX_PTY_RESUMES))
        );
    }

    #[test]
    fn a_lowered_depth_limit_keeps_a_deep_agent_exited() {
        let mut deep = row(true);
        deep.depth = 3;
        assert_eq!(
            restart_skip(&deep, false, 2),
            Some(Skip::TooDeep { depth: 3, max: 2 })
        );
        // A desktop's own launch has no spawner, so no depth applies.
        deep.spawned_by = None;
        assert_eq!(restart_skip(&deep, false, 2), None);
    }

    #[cfg(unix)]
    mod guard {
        use std::collections::BTreeMap;
        use std::ffi::OsString;
        use std::path::Path;
        use std::time::Duration;

        use hyprlayer_models::{PtyAgentState, PtyLaunchSession};

        use super::super::{ResumeCause, Resumed};
        use crate::claude::memory::project_key;
        use crate::env::{RuntimeEnvironment, RuntimePaths};
        use crate::runtime::builder::RuntimeBuilder;
        use crate::runtime::terminals::{CloseTerminalRequest, LaunchAgentRequest};
        use crate::runtime::HyprlayerRuntime;
        use crate::store::{NewWorkstream, PtyAgentRow, WorktreeMode};

        const SID: &str = "conv-1";

        /// A claude that records each launch's argv, then stays up.
        fn stub(root: &Path) -> String {
            use std::os::unix::fs::PermissionsExt;
            let path = root.join("claude-stub");
            let log = root.join("launches");
            std::fs::write(
                &path,
                format!(
                    "#!/bin/sh\necho \"$@\" >> '{}'\nexec sleep 30\n",
                    log.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path.display().to_string()
        }

        async fn runtime(root: &Path) -> std::sync::Arc<HyprlayerRuntime> {
            let home = root.join("home");
            std::fs::create_dir_all(&home).unwrap();
            let mut process_env = BTreeMap::new();
            if let Some(path) = std::env::var_os("PATH") {
                process_env.insert(OsString::from("PATH"), path);
            }
            let environment = RuntimeEnvironment {
                home_dir: home.clone(),
                process_cwd: home,
                process_env,
                env_overlay: Default::default(),
            };
            let paths = RuntimePaths {
                hyprlayer_config_file: root.join("hyprlayer-config.json"),
                app_config_dir: root.join("app-config"),
                app_data_dir: root.join("app-data"),
                default_worktree_root: root.join("worktrees"),
                default_repos_root: root.join("repos"),
                opencode_config_file: root.join("opencode.json"),
                opencode_auth_file: root.join("opencode-auth.json"),
                opencode_data_dir: root.join("opencode-data"),
                claude_config_dir: root.join("claude"),
                claude_managed_settings_files: Vec::new(),
                codex_home: root.join("codex"),
                agent_roots: Vec::new(),
                download_dir: root.join("downloads"),
            };
            RuntimeBuilder::new(environment, paths)
                .open()
                .await
                .unwrap()
        }

        /// A stored agent cut off mid-turn, its conversation on disk, its engine the stub.
        async fn stored_agent(rt: &HyprlayerRuntime, root: &Path) -> String {
            let command = stub(root);
            rt.settings
                .update(|s| {
                    let engine = s
                        .multiplexer
                        .engines
                        .iter_mut()
                        .find(|e| e.id == "claude-code")
                        .unwrap();
                    engine.command = command;
                    engine.args = Vec::new();
                })
                .await
                .unwrap();
            let worktree = root.join("wt");
            std::fs::create_dir_all(&worktree).unwrap();
            let worktree = worktree.display().to_string();
            let ws = rt
                .db
                .insert_workstream(NewWorkstream {
                    project_path: root.display().to_string(),
                    title: "t".into(),
                    slug: "t-1".into(),
                    branch: "feat".into(),
                    worktree_path: worktree.clone(),
                    base_branch: "origin/main".into(),
                    worktree_mode: WorktreeMode::Isolated,
                })
                .unwrap();
            let transcripts = rt
                .environment
                .home_dir
                .join(".claude")
                .join("projects")
                .join(project_key(&worktree));
            std::fs::create_dir_all(&transcripts).unwrap();
            std::fs::write(transcripts.join(format!("{SID}.jsonl")), "").unwrap();
            rt.db
                .upsert_pty_agent(&PtyAgentRow {
                    terminal_id: "t1".into(),
                    workstream_id: ws.id,
                    engine_id: "claude-code".into(),
                    launch: PtyLaunchSession::default(),
                    spawned_by: None,
                    depth: 0,
                    mode: None,
                    provider_session_id: Some(SID.into()),
                    title: Some("Long task".into()),
                    working: true,
                    resumes: 0,
                })
                .unwrap();
            "t1".into()
        }

        async fn eventually(check: impl Fn() -> bool) -> bool {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
            while !check() {
                if tokio::time::Instant::now() > deadline {
                    return false;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            true
        }

        fn launches(root: &Path) -> Vec<String> {
            std::fs::read_to_string(root.join("launches"))
                .map(|t| t.lines().map(str::to_string).collect())
                .unwrap_or_default()
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn a_restart_and_a_message_racing_to_resume_launch_it_once() {
            let root = tempfile::tempdir().unwrap();
            let rt = runtime(root.path()).await;
            let id = stored_agent(&rt, root.path()).await;
            let control = rt.agent_control();

            let (restart, message) = tokio::join!(
                control.resume_pty(&id, ResumeCause::Restart),
                control.resume_pty(&id, ResumeCause::Message("carry on".into())),
            );
            let outcomes = [restart.unwrap(), message.unwrap()];
            let launched = outcomes
                .iter()
                .filter(|o| matches!(o, Resumed::Launched))
                .count();
            let live = outcomes
                .iter()
                .filter(|o| matches!(o, Resumed::Live))
                .count();
            assert_eq!((launched, live), (1, 1));

            let started = eventually(|| !launches(root.path()).is_empty()).await;
            assert!(started, "the stub never launched");
            tokio::time::sleep(Duration::from_millis(300)).await;
            let argv = launches(root.path());
            assert_eq!(argv.len(), 1, "one process for the conversation: {argv:?}");
            assert!(argv[0].contains(&format!("--resume {SID}")), "{argv:?}");
            let snapshot = rt.pty_agents.snapshot(&id).expect("live under its own id");
            assert_ne!(snapshot.agent.state, PtyAgentState::Exited);
            assert_eq!(snapshot.agent.title.as_deref(), Some("Long task"));

            // A third caller, after it is up, finds it live and launches nothing.
            assert!(matches!(
                control.resume_pty(&id, ResumeCause::Restart).await.unwrap(),
                Resumed::Live
            ));
            rt.shutdown().await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn a_desktop_resuming_the_conversation_replaces_the_stored_agent() {
            let root = tempfile::tempdir().unwrap();
            let rt = runtime(root.path()).await;
            let id = stored_agent(&rt, root.path()).await;
            let ws = rt.db.pty_agent(&id).unwrap().unwrap().workstream_id;
            let (_, launched) = rt
                .terminals()
                .launch_agent(LaunchAgentRequest {
                    workstream_id: ws,
                    engine_id: "claude-code".into(),
                    session: PtyLaunchSession {
                        session_id: Some(SID.into()),
                        resume: true,
                        ..Default::default()
                    },
                    cols: 80,
                    rows: 24,
                    lineage: None,
                    resume_of: None,
                })
                .await
                .unwrap();
            assert_ne!(launched.terminal_id, id);
            assert!(rt.db.pty_agent(&id).unwrap().is_none());
            assert!(rt
                .agent_control()
                .resume_pty(&id, ResumeCause::Restart)
                .await
                .is_err());
            assert!(eventually(|| launches(root.path()).len() == 1).await);
            tokio::time::sleep(Duration::from_millis(300)).await;
            assert_eq!(launches(root.path()).len(), 1);
            rt.shutdown().await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn a_closed_tab_is_never_resumed() {
            let root = tempfile::tempdir().unwrap();
            let rt = runtime(root.path()).await;
            let id = stored_agent(&rt, root.path()).await;
            rt.terminals()
                .close(CloseTerminalRequest {
                    terminal_id: id.clone(),
                })
                .await
                .unwrap();
            assert!(rt.db.pty_agent(&id).unwrap().is_none());
            let control = rt.agent_control();
            assert!(control.resume_pty(&id, ResumeCause::Restart).await.is_err());
            assert!(launches(root.path()).is_empty());
            rt.shutdown().await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
        async fn an_archived_workstream_drops_the_agent_unlaunched() {
            let root = tempfile::tempdir().unwrap();
            let rt = runtime(root.path()).await;
            let id = stored_agent(&rt, root.path()).await;
            let ws = rt.db.pty_agent(&id).unwrap().unwrap().workstream_id;
            rt.db
                .with(|conn| {
                    conn.execute(
                        "UPDATE workstreams SET status = 'archived' WHERE id = ?1",
                        [&ws],
                    )
                })
                .unwrap();
            let outcome = rt
                .agent_control()
                .resume_pty(&id, ResumeCause::Restart)
                .await
                .unwrap();
            assert!(matches!(
                outcome,
                Resumed::Skipped(super::super::Skip::Archived)
            ));
            rt.pty_agents.flush_store();
            assert!(rt.db.pty_agent(&id).unwrap().is_none());
            assert!(launches(root.path()).is_empty());
            rt.shutdown().await;
        }
    }

    #[test]
    fn no_conversation_means_nothing_to_resume() {
        let mut fresh = row(true);
        fresh.provider_session_id = None;
        assert_eq!(restart_skip(&fresh, false, 3), Some(Skip::NoConversation));
    }
}
