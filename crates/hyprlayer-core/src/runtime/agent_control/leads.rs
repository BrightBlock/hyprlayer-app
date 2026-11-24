use std::collections::HashSet;
use std::path::Path;

use hyprlayer_models::{AgentCaller, ClientRef, Harness, LeadView, WorktreeMode};

use super::{blocking, AgentControlService};
use crate::error::{CoreError, Result};
use crate::gitops;
use crate::pipeline::Stage;
use crate::runtime::sessions::{
    LaunchSessionParams, SendContext, SessionLineage, StartStageSessionRequest,
};
use crate::runtime::workstreams::{CreateWorkstreamRequest, ListWorkstreamsRequest};
use crate::store::{Session, Workstream};

pub(super) const LEAD_TITLE: &str = "Lead Engineer";

impl AgentControlService<'_> {
    pub async fn list_leads(&self) -> Result<Vec<LeadView>> {
        let db = self.runtime.db.clone();
        let projects = blocking(move || db.list_projects()).await?;
        let mut leads = Vec::new();
        for project in projects.into_iter().filter(|p| p.source.is_git()) {
            leads.push(self.lead_view(project.path).await?);
        }
        Ok(leads)
    }

    async fn lead_view(&self, project_path: String) -> Result<LeadView> {
        let active = self.active_workstreams(&project_path).await?;
        let main = active.iter().find(|w| w.is_inplace()).map(|w| w.id.clone());
        let db = self.runtime.db.clone();
        let path = project_path.clone();
        let links = blocking(move || db.lead_links(&path)).await?;
        let lead_links: Vec<_> = links
            .iter()
            .filter(|l| l.lead && main.as_deref() == Some(l.workstream_id.as_str()))
            .collect();
        let leads: HashSet<&str> = lead_links.iter().map(|l| l.id.as_str()).collect();
        let mut driven: HashSet<String> = links
            .iter()
            .filter(|l| l.spawn_parent.as_deref().is_some_and(|p| leads.contains(p)))
            .map(|l| l.workstream_id.clone())
            .collect();
        driven.extend(
            leads
                .iter()
                .flat_map(|lead| self.runtime.agent_links.children_of(lead))
                .filter_map(|child| self.runtime.pty_agents.snapshot(&child))
                .map(|pty| pty.agent.workstream_id),
        );
        let driving = active
            .iter()
            .filter(|w| main.as_deref() != Some(w.id.as_str()) && driven.contains(&w.id))
            .map(|w| w.id.clone())
            .collect();
        let session_id = lead_links
            .iter()
            .max_by(|a, b| a.started_at.total_cmp(&b.started_at))
            .map(|l| l.id.clone());
        let lead = self.runtime.settings.get().await.agent_control.lead();
        let remote_control = lead.remote_control.contains(&project_path);
        let path = project_path.clone();
        let project_rules = blocking(move || {
            Ok(crate::lead::rules_in_effect(&lead, &path).map(|_| {
                crate::lead::project_rules_path(Path::new(&path))
                    .display()
                    .to_string()
            }))
        })
        .await?;
        let live_children = match &session_id {
            Some(id) => {
                let caller = AgentCaller::Session {
                    session_id: id.clone(),
                };
                self.live_children(&caller).await? as u32
            }
            None => 0,
        };
        let remote_control_url = match &session_id {
            Some(id) if remote_control && self.runtime.sessions().is_live(id).await => {
                self.runtime.remote_control_url(id)
            }
            _ => None,
        };
        let summary = session_id
            .as_deref()
            .and_then(|id| self.runtime.lead_summaries.get(id));
        Ok(LeadView {
            workstream_id: main,
            session_id,
            project_path,
            driving,
            live_children,
            project_rules,
            remote_control,
            remote_control_url,
            summary,
        })
    }

    pub async fn set_lead_remote_control(
        &self,
        project_path: &str,
        enabled: bool,
    ) -> Result<LeadView> {
        let remember = |on: bool| {
            let path = project_path.to_string();
            self.runtime.settings.update(move |s| {
                let mut lead = s.agent_control.lead();
                if on {
                    lead.remote_control.insert(path);
                } else {
                    lead.remote_control.remove(&path);
                }
                s.agent_control.lead = Some(lead);
            })
        };
        remember(enabled).await?;
        if let Some(id) = self.lead_view(project_path.to_string()).await?.session_id {
            let applied = self
                .runtime
                .sessions()
                .apply_remote_control(&id, project_path, enabled)
                .await;
            if let Err(e) = applied {
                remember(!enabled).await?;
                return Err(e);
            }
        }
        self.lead_view(project_path.to_string()).await
    }

    pub async fn message_lead(
        &self,
        project_path: &str,
        text: &str,
        author: Option<ClientRef>,
    ) -> Result<String> {
        let settings = self.runtime.settings.get().await.agent_control.lead();
        let mode = settings.permission_mode();
        if !settings.enabled {
            return Err(CoreError::LeadDisabled);
        }
        if !self
            .runtime
            .projects()
            .source_of(project_path)
            .await
            .is_git()
        {
            return Err(CoreError::LeadNeedsGit {
                path: project_path.to_string(),
            });
        }
        let _one_lead = self
            .runtime
            .agent_spawn_locks
            .acquire(&format!("lead:{project_path}"))
            .await;
        let main = self.main_workstream(project_path).await?;
        let sessions = self.runtime.sessions();
        if let Some(session) = newest_lead(&sessions.list(&main.id).await?).cloned() {
            let id = session.id.clone();
            {
                let op = sessions.op_lock(&id).await;
                let _resuming = op.lock().await;
                if !sessions.is_live(&id).await {
                    sessions
                        .launch(LaunchSessionParams {
                            permission_mode: Some(
                                session
                                    .requested_permission_mode
                                    .map_or(mode, |chosen| super::spawn::lower(chosen, mode)),
                            ),
                            session,
                            project_path: main.project_path,
                            model: None,
                            effort: None,
                            resume: true,
                            boundary_on_relaunch: true,
                            max_budget_usd: None,
                            extra_allowed_tools: Vec::new(),
                            extra_disallowed_tools: Vec::new(),
                            report_only: false,
                        })
                        .await?;
                }
            }
            self.note_human_turn(&id);
            sessions
                .send(
                    &id,
                    text,
                    &[],
                    None,
                    None,
                    SendContext {
                        base_rev: None,
                        author,
                        sender: None,
                    },
                )
                .await?;
            return Ok(id);
        }
        let started = sessions
            .prepare_start_with(
                StartStageSessionRequest {
                    workstream: main,
                    stage: Stage::General,
                    input: text.to_string(),
                    attached_docs: Vec::new(),
                    harness: Some(Harness::Claude),
                    model: settings.model,
                    effort: settings.effort,
                    permission_mode: Some(mode),
                    prompt_name: None,
                    client_message_id: None,
                    parent_session_id: None,
                },
                SessionLineage::lead(),
            )
            .await?;
        let id = started.session.id.clone();
        sessions.launch_and_converse(started.launch, author).await?;
        Ok(id)
    }

    async fn main_workstream(&self, project_path: &str) -> Result<Workstream> {
        let active = self.active_workstreams(project_path).await?;
        if let Some(main) = active.into_iter().find(Workstream::is_inplace) {
            return Ok(main);
        }
        let checkout = project_path.to_string();
        let branch = blocking(move || Ok(gitops::current_branch(Path::new(&checkout))))
            .await?
            .ok_or_else(|| CoreError::LeadNeedsBranch {
                path: project_path.to_string(),
            })?;
        let created = self
            .runtime
            .workstreams()
            .create(CreateWorkstreamRequest {
                project_path: project_path.to_string(),
                title: LEAD_TITLE.into(),
                base: None,
                mode: WorktreeMode::Inplace,
                branch: Some(branch),
                branch_prefix: None,
                parent_workstream_id: None,
            })
            .await?;
        Ok(created.item)
    }

    async fn active_workstreams(&self, project_path: &str) -> Result<Vec<Workstream>> {
        self.runtime
            .workstreams()
            .list(ListWorkstreamsRequest {
                project_path: project_path.to_string(),
                include_archived: false,
            })
            .await
    }
}

fn newest_lead(sessions: &[Session]) -> Option<&Session> {
    sessions
        .iter()
        .filter(|s| s.lead && s.archived_at.is_none())
        .max_by(|a, b| a.started_at.total_cmp(&b.started_at))
}
