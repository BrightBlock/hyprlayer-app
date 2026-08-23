//! Pure reader over a `Workflow` tool call's on-disk output tree.
//!
//! A `Workflow` tool_result names a run directory
//! (`<session>/subagents/workflows/wf_<id>/`) holding one `journal.jsonl` (a
//! `started`/`result` bracket per subagent) and one `agent-<id>.jsonl` per
//! subagent — that agent's own `stream-json`-shaped lines, the same wire
//! format the driver's live stdout carries. None of it reaches the driver:
//! a workflow agent's output is never printed to the parent `claude`
//! process, so nothing about it exists in [`super::events::SessionEvent`]
//! until something reads these files directly. This module is that reader.
//!
//! Per run, [`WorkflowRuns::read_tick`] tails `journal.jsonl` and every
//! `agent-*.jsonl` from a stored byte offset and synthesizes events: one
//! `ToolUse`/`ToolResult` pair per agent (its synthetic parent, nested under
//! the `Workflow` row via `workflow_tool_use_id`), plus that agent's own
//! `ToolUse`/`ToolResult` events (nested under its synthetic parent),
//! reusing [`super::protocol::parse_line`] and
//! [`super::driver::parsed_to_events`] so an agent's own transcript parses
//! identically to the main session's.
//!
//! No IPC, no Tauri, no task: every function here is pure over paths and
//! bytes, or a method on [`WorkflowRuns`]/[`WorkflowRun`] mutating only
//! their own in-memory cursors. Deciding *when* to call [`WorkflowRuns::read_tick`]
//! is somebody else's problem, as is logging a [`workflow_launch_from_result`]
//! failure — this module only reports `None`.
//!
//! [`watch_workflows`] is the one exception, and only just: it arms a
//! `notify` debouncer over a session's `subagents/workflows/` directory and
//! signals a [`Notify`] when something under it actually changed. It still
//! spawns nothing — the caller owns the reader task and hands its handle
//! back with [`WorkflowWatch::attach_reader`], so core keeps assuming no
//! ambient runtime (ADR-0001). [`SessionWorkflows`] bundles the two halves
//! per live session: the runs being tailed, plus that session's single
//! watch.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{
    new_debouncer, DebounceEventResult, DebouncedEvent, Debouncer, RecommendedCache,
};
use serde_json::{json, Value};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use super::driver::parsed_to_events;
use super::events::SessionEvent;
use super::protocol::{self, ParsedLine};
use crate::error::{CoreError, Result};
use crate::index::watcher::{notify_to_core, signals_change};

/// The run a `Workflow` tool_result announces. Its content, verbatim from a
/// real launch:
///
/// ```text
/// Workflow launched in background. Task ID: whjv5o0xq
/// Summary: Map how hyprlayer-app drives spec-driven dev stages, and what
/// each transition actually writes
/// Transcript dir: /home/jt/.claude/projects/<slug>/<session>/subagents/workflows/wf_89a9bc27-4f1
/// Script file: /home/jt/.claude/projects/<slug>/<session>/workflows/scripts/map-stage-driving-wf_89a9bc27-4f1.js
/// (Edit this file with Write/Edit and re-invoke Workflow with {scriptPath: "…"} to iterate without resending the script.)
/// Run ID: wf_89a9bc27-4f1
/// To resume after editing the script: Workflow({scriptPath: "…", resumeFromRunId: "wf_89a9bc27-4f1"}) — completed agents return cached results (…).
///
/// You will be notified when it completes. Use /workflows to watch live progress.
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowLaunch {
    pub run_dir: PathBuf,
    pub run_id: String,
    pub task_id: String,
}

/// Parse a `Workflow` tool_result into a [`WorkflowLaunch`], or `None` if
/// this isn't one. Matched by field prefix (`Task ID:`, `Transcript dir:`,
/// `Run ID:`), never by line position: the `Summary:` line is
/// model-authored and can wrap across several lines, so a caller that
/// counted lines would misread everything after it.
///
/// `run_id` is cross-checked against `run_dir`'s own basename; disagreement
/// means the wire format moved under us, so this degrades to `None` rather
/// than trust a pairing it can't verify.
pub fn workflow_launch_from_result(content: &str) -> Option<WorkflowLaunch> {
    let mut task_id = None;
    let mut run_dir = None;
    let mut run_id = None;
    for line in content.lines() {
        if let Some((_, rest)) = line.split_once("Task ID:") {
            task_id.get_or_insert_with(|| rest.trim().to_string());
        }
        if let Some((_, rest)) = line.split_once("Transcript dir:") {
            run_dir.get_or_insert_with(|| PathBuf::from(rest.trim()));
        }
        if let Some((_, rest)) = line.split_once("Run ID:") {
            run_id.get_or_insert_with(|| rest.trim().to_string());
        }
    }
    let run_dir = run_dir?;
    let run_id = run_id?;
    let task_id = task_id?;
    if run_dir.file_name().and_then(|n| n.to_str()) != Some(run_id.as_str()) {
        return None;
    }
    Some(WorkflowLaunch {
        run_dir,
        run_id,
        task_id,
    })
}

/// Everything one live workflow run needs, keyed by run id in
/// [`WorkflowRuns`]. Owned in core so the reader stays testable; the Tauri
/// layer holds the owning [`WorkflowRuns`] behind an `Arc<Mutex<_>>` and
/// hands a clone of the handle to the watcher's reader task.
#[derive(Debug)]
pub struct WorkflowRun {
    pub run_dir: PathBuf,
    pub workflow_tool_use_id: String,
    /// The forwarder `seq` of the `ToolResult` that armed this run, so a
    /// rewind can drop the runs whose parent row it cut:
    /// `runs.retain(|_, r| r.armed_seq < cutoff)`.
    pub armed_seq: i64,
    /// Byte offset per file, keyed by absolute path. Absolute paths are
    /// disjoint across concurrent runs and concurrent sessions, so
    /// `journal.jsonl` shares this map with every `agent-*.jsonl` and needs
    /// no separate handling.
    offsets: HashMap<PathBuf, u64>,
    /// Agents whose synthetic parent `ToolUse` has been emitted and whose
    /// matching `ToolResult` hasn't landed yet — i.e. currently open.
    opened: HashSet<String>,
    /// Agents seen via a journal `started` line but not yet in `opened`:
    /// nothing guarantees the CLI has written the agent's own file (or its
    /// prompt line) by the time `started` lands, so opening is deferred
    /// until a complete prompt line is readable, and retried every tick. A
    /// placeholder label can't be corrected once persisted, so this is a
    /// wait, not a guess.
    pending_open: HashSet<String>,
    /// Ids already emitted, **split by kind**. `SessionEvent::ToolUse.id`
    /// and `ToolResult.tool_use_id` are the same string
    /// (`events.rs:229-241`), so one set cannot tell "I emitted the use"
    /// from "I emitted the result": after a re-arm it would suppress the
    /// closing result of every tool whose use was persisted before the
    /// restart, leaving rows — including the synthetic per-agent parents —
    /// open forever.
    ///
    /// Seeded from the persisted log on a re-arm (Phase 4 §7) and only read
    /// here: within one reader's life the byte cursors already guarantee a
    /// line is processed once, so nothing needs recording on emit.
    seen_uses: HashSet<String>,
    seen_results: HashSet<String>,
}

/// Every workflow run one live session is currently tailing.
///
/// **Per session, not global.** The Tauri layer holds one of these on each
/// `LiveSession` (Phase 4 §8/§10): [`read_tick`](Self::read_tick) drains
/// every run in the map it is called on, so a single shared map would push
/// two concurrently-running workstreams' agents into whichever forwarder
/// happened to tick — cross-attaching one session's subagent rows onto the
/// other's transcript.
#[derive(Debug, Default)]
pub struct WorkflowRuns {
    runs: HashMap<String, WorkflowRun>,
}

impl WorkflowRuns {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start tracking a run. `workflow_tool_use_id` is the id of the
    /// `Workflow` tool_use this launch answers (the caller already has it —
    /// it's the `tool_use_id` on the very `ToolResult` that parsed into
    /// `launch`), and becomes every one of this run's agents' `parent_id`,
    /// which is what nests them under the `Workflow` row.
    /// `armed_seq` is that `ToolResult`'s own forwarder `seq`.
    ///
    /// **A run id already present is left entirely untouched.** A rewind
    /// re-registers runs it did not cut while the reader task — and its
    /// advanced offsets — survive, so a re-register that reset the cursors
    /// would re-read the whole run from byte 0.
    pub fn register(
        &mut self,
        launch: WorkflowLaunch,
        workflow_tool_use_id: String,
        armed_seq: i64,
    ) {
        self.register_seeded(
            launch,
            workflow_tool_use_id,
            armed_seq,
            HashSet::new(),
            HashSet::new(),
        );
    }

    /// [`register`](Self::register), seeding the run's dedup sets from a
    /// persisted event log so a re-armed reader re-emits only what is
    /// missing from it (Phase 4 §7). Returns whether it actually inserted —
    /// `false` means the run id was already present and **the seeds were
    /// discarded**, which is correct (a surviving reader's byte cursors
    /// already cover everything the seeds describe) but worth asserting on
    /// rather than assuming.
    ///
    /// The seeds go in *here*, as arguments, rather than through a setter
    /// called afterwards: [`read_tick`](Self::read_tick) consults them at
    /// emit time and nothing ever writes back into them, so a wake landing
    /// between an unseeded insert and a later seeding call re-emits the
    /// whole run as duplicates — with the byte cursors already advanced past
    /// them, so the seeds could never catch up.
    pub fn register_seeded(
        &mut self,
        launch: WorkflowLaunch,
        workflow_tool_use_id: String,
        armed_seq: i64,
        seen_uses: HashSet<String>,
        seen_results: HashSet<String>,
    ) -> bool {
        let mut inserted = false;
        self.runs.entry(launch.run_id).or_insert_with(|| {
            inserted = true;
            WorkflowRun {
                run_dir: launch.run_dir,
                workflow_tool_use_id,
                armed_seq,
                offsets: HashMap::new(),
                opened: HashSet::new(),
                pending_open: HashSet::new(),
                seen_uses,
                seen_results,
            }
        });
        inserted
    }

    /// Drop every run whose arming `ToolResult` a rewind just cut, i.e.
    /// `runs.retain(|_, r| r.armed_seq < cutoff)` against the same cutoff
    /// handed to `cut_session_events_from`, which deletes `seq >= cutoff`.
    /// A run kept here keeps its advanced byte cursors, which is the whole
    /// reason the map outlives the forwarder that armed it.
    pub fn retain_armed_below(&mut self, cutoff: i64) {
        self.runs.retain(|_, run| run.armed_seq < cutoff);
    }

    /// How many runs this session is tailing.
    pub fn len(&self) -> usize {
        self.runs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// Read whatever is newly available across every registered run and
    /// return the resulting events, in no particular cross-run order (the
    /// forwarder assigns `seq` on arrival, per-run order is preserved).
    pub fn read_tick(&mut self) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        for (run_id, run) in self.runs.iter_mut() {
            run.read_tick(run_id, &mut events);
        }
        events
    }
}

impl WorkflowRun {
    /// `journal.jsonl` first, then every agent file, so an agent's tool
    /// events never precede the synthetic parent that adopts them. Nesting
    /// itself is order-independent (`childrenByParent`,
    /// `TranscriptView.tsx:1167-1176`), but a child whose `parentId` names
    /// an item not yet in the log is filtered out of `topLevel` and stays
    /// invisible until the parent lands.
    fn read_tick(&mut self, run_id: &str, events: &mut Vec<SessionEvent>) {
        self.read_journal(run_id, events);
        self.drain_known_agents(run_id, events);
    }

    /// `journal.jsonl`'s new lines: `{type,key,agentId}` on `started`,
    /// `{type,key,agentId,result}` on `result` (`result` a plain string).
    /// Both branches drain that one agent's own file immediately rather than
    /// leaving it to [`Self::drain_known_agents`] below: on an
    /// already-finished run (attach or resume after the whole thing
    /// completed) `started` and `result` for the same agent land in this
    /// SAME batch, and `close` stops tracking the agent, so without an
    /// eager drain right after `started` the agent would open with the
    /// bare-id fallback label and none of its own tool calls would ever be
    /// read — a needlessly empty render for data sitting right there on
    /// disk.
    fn read_journal(&mut self, run_id: &str, events: &mut Vec<SessionEvent>) {
        let path = self.run_dir.join("journal.jsonl");
        let Some(lines) = read_new_lines(&path, &mut self.offsets) else {
            return;
        };
        for line in &lines {
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let Some(agent_id) = v.get("agentId").and_then(Value::as_str) else {
                continue;
            };
            match v.get("type").and_then(Value::as_str) {
                Some("started") => {
                    if !self.opened.contains(agent_id) {
                        self.pending_open.insert(agent_id.to_string());
                        self.drain_one(run_id, agent_id, events);
                    }
                }
                Some("result") => {
                    self.drain_one(run_id, agent_id, events);
                    let result = v.get("result").and_then(Value::as_str).unwrap_or_default();
                    self.close(run_id, agent_id, result, events);
                }
                _ => {}
            }
        }
    }

    /// [`Self::drain_one`] for every agent this run currently knows about
    /// (opened, or still waiting on a readable prompt line).
    fn drain_known_agents(&mut self, run_id: &str, events: &mut Vec<SessionEvent>) {
        let agent_ids: Vec<String> = self
            .opened
            .iter()
            .chain(self.pending_open.iter())
            .cloned()
            .collect();
        for agent_id in agent_ids {
            self.drain_one(run_id, &agent_id, events);
        }
    }

    /// Read whatever is newly available in one agent's own file and turn it
    /// into events, opening the agent's synthetic parent the moment its
    /// prompt line becomes readable. A no-op (cheap: one failed or empty
    /// read) when there's nothing new, so calling this twice in the same
    /// tick for the same agent — `read_journal` does, around both `started`
    /// and `result` — never double-processes anything.
    fn drain_one(&mut self, run_id: &str, agent_id: &str, events: &mut Vec<SessionEvent>) {
        let path = self.run_dir.join(format!("agent-{agent_id}.jsonl"));
        let Some(lines) = read_new_lines(&path, &mut self.offsets) else {
            return; // the CLI hasn't created this agent's file yet
        };
        let parent_id = format!("wf:{run_id}:{agent_id}");
        for line in &lines {
            let prompt = is_prompt_line(line);
            if prompt && self.pending_open.contains(agent_id) {
                let label =
                    agent_label_from_prompt_line(line).unwrap_or_else(|| agent_id.to_string());
                self.open(run_id, agent_id, label, events);
            }
            events.extend(keep_new_tool_events(
                parse_agent_line(line, &parent_id, prompt),
                &self.seen_uses,
                &self.seen_results,
            ));
        }
    }

    /// Emit the synthetic per-agent `ToolUse`. Idempotent: a second call for
    /// an already-open agent (`close`'s bare-id fallback racing this tick's
    /// label resolution) does not double-emit, and neither does an id a
    /// re-arm already found in the persisted log.
    fn open(
        &mut self,
        run_id: &str,
        agent_id: &str,
        label: String,
        events: &mut Vec<SessionEvent>,
    ) {
        self.pending_open.remove(agent_id);
        let tool_id = format!("wf:{run_id}:{agent_id}");
        if self.opened.insert(agent_id.to_string()) && !self.seen_uses.contains(&tool_id) {
            events.push(SessionEvent::ToolUse {
                id: tool_id,
                name: "Agent".into(),
                input: json!({ "description": label }).to_string(),
                parent_id: Some(self.workflow_tool_use_id.clone()),
            });
        }
    }

    /// Close an agent on its journal `result` line. `read_journal` already
    /// called `drain_one` for this agent immediately before this, so `open`
    /// below only actually fires the bare-agent-id fallback on a genuine,
    /// still-unresolved race (the file never became readable at all before
    /// `result` landed — see `pending_open`'s doc); otherwise it's a no-op,
    /// the agent is already open with its real label.
    ///
    /// The `ToolResult` is gated on `seen_results` alone, never on whether
    /// the matching `ToolUse` was already emitted — that is the whole point
    /// of splitting the two sets. Then stop tracking the agent entirely, so
    /// a belatedly-readable file is never processed into a second,
    /// contradictory open on an already-closed row.
    fn close(
        &mut self,
        run_id: &str,
        agent_id: &str,
        result: &str,
        events: &mut Vec<SessionEvent>,
    ) {
        let tool_id = format!("wf:{run_id}:{agent_id}");
        if !self.opened.contains(agent_id) {
            self.open(run_id, agent_id, agent_id.to_string(), events);
        }
        if !self.seen_results.contains(&tool_id) {
            events.push(SessionEvent::ToolResult {
                tool_use_id: tool_id,
                content: result.to_string(),
                is_error: false,
                parent_id: Some(self.workflow_tool_use_id.clone()),
            });
        }
        self.opened.remove(agent_id);
        self.pending_open.remove(agent_id);
    }
}

/// One live session's workflow state: every run it is tailing, plus the
/// **single** recursive watch covering that session's
/// `<session>/subagents/workflows/` directory.
///
/// One watch per session, not per run, which bounds the watcher count by
/// live sessions rather than by workflows launched — notify arms new
/// subdirectories under a recursive watch itself
/// (`index/watcher.rs:182-183`), so the second and later `wf_*` of a session
/// are covered by the watch the first one armed.
///
/// The Tauri layer owns this behind an `Arc<Mutex<_>>` on its `LiveSession`,
/// so leaving the registry drops it, which stops both the debouncer and the
/// reader task with no explicit teardown.
#[derive(Debug, Default)]
pub struct SessionWorkflows {
    pub runs: WorkflowRuns,
    /// `None` until this session's first `Workflow` launch arms it.
    armed: Option<(WorkflowWatch, Arc<Notify>)>,
}

impl SessionWorkflows {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_armed(&self) -> bool {
        self.armed.is_some()
    }

    /// Arm this session's watch if it has none, and return the [`Notify`] its
    /// callback signals plus **whether this call did the arming**. Only then
    /// does the caller spawn a reader task and hand the handle back through
    /// [`attach_reader`](Self::attach_reader); a second workflow in the same
    /// session gets the same `Notify` and the same reader.
    pub fn arm(
        &mut self,
        workflows_dir: &Path,
        on_change: Box<dyn Fn() + Send + Sync>,
    ) -> Result<(Arc<Notify>, bool)> {
        if let Some((_, notify)) = &self.armed {
            return Ok((notify.clone(), false));
        }
        let (watch, notify) = watch_workflows(workflows_dir, on_change)?;
        self.armed = Some((watch, notify.clone()));
        Ok((notify, true))
    }

    /// Give the watch the reader task it wakes, so dropping one stops both.
    /// A handle handed to an unarmed session is aborted on the spot: nothing
    /// would ever signal it, so it would park on `notified()` forever.
    ///
    /// **The reader must hold a `Weak` to this `SessionWorkflows`, not an
    /// `Arc`.** Its handle lands inside the object it reads, and
    /// [`WorkflowWatch`]'s `Drop` is the only thing that aborts it, so a strong
    /// handle in the task makes that `Drop` unreachable — the session leaves
    /// the registry, the debouncer thread and its recursive inotify watch stay
    /// armed on a dead session's tree, and the task parks forever because a
    /// dead claude appends nothing more to fire the watch.
    pub fn attach_reader(&mut self, handle: JoinHandle<()>) {
        match &mut self.armed {
            Some((watch, _)) => watch.attach_reader(handle),
            None => handle.abort(),
        }
    }
}

/// A live `notify` watch over one session's `subagents/workflows/` tree,
/// plus the reader task it wakes. Dropping it stops both — which holds only
/// as long as the reader itself holds no strong reference back to the
/// [`SessionWorkflows`] this lives on (see [`SessionWorkflows::attach_reader`]).
///
/// Deliberately not [`crate::index::watcher::WatchHandle`]: that type's
/// `_debouncer` field is private and `claude::workflow` is not a descendant
/// of `index::watcher`, so constructing one here would not compile.
pub struct WorkflowWatch {
    _debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
    reader: Option<JoinHandle<()>>,
}

impl WorkflowWatch {
    /// Record the reader task this watch signals, aborting any task it
    /// replaces.
    pub fn attach_reader(&mut self, handle: JoinHandle<()>) {
        if let Some(old) = self.reader.replace(handle) {
            old.abort();
        }
    }
}

impl Drop for WorkflowWatch {
    /// Both halves stop non-blockingly: `JoinHandle::abort` only sets a flag,
    /// and `Debouncer`'s own `Drop` is a single relaxed atomic store. That
    /// matters because the Tauri layer drops this from inside its session
    /// registry — never add an await, a join, or a blocking send here.
    fn drop(&mut self) {
        if let Some(reader) = self.reader.take() {
            reader.abort();
        }
    }
}

impl std::fmt::Debug for WorkflowWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkflowWatch")
            .field("reader", &self.reader.is_some())
            .finish_non_exhaustive()
    }
}

/// Arm one debounced recursive watch over `workflows_dir`
/// (`<session>/subagents/workflows/`), signalling the returned [`Notify`]
/// once per debounced batch that actually mutated something.
///
/// The `Notify` is the production wakeup: the callback is `Fn` while
/// [`WorkflowRuns::read_tick`] takes `&mut self`, and megabyte-scale file
/// reads and JSON parsing on the debouncer thread would block notify's own
/// event loop across up to 34 concurrently-appended files. So the callback
/// signals and a task owned by the caller does the work, coalescing a burst
/// into one pass. `on_change` fires on the same edge for a caller that wants
/// to observe batches without a runtime — the `#[ignore]`d end-to-end test
/// below counts them; production passes a no-op.
///
/// One recursive watch, not a [`crate::index::watcher`]-style pruned plan:
/// this tree is small, entirely ours to read, and grows a `wf_*`
/// subdirectory per launch that notify arms itself
/// (`index/watcher.rs:182-183`).
pub fn watch_workflows(
    workflows_dir: &Path,
    on_change: Box<dyn Fn() + Send + Sync>,
) -> Result<(WorkflowWatch, Arc<Notify>)> {
    // Create the directory before arming it. `debouncer.watch()` on a
    // nonexistent path is a `notify::Error`, and the run tree does not exist
    // yet when the launch tool_result lands: measured against two real
    // sessions, `subagents/workflows/` and the `wf_*` under it are born at
    // the same timestamp, +8 ms and +45 ms after the tool_result. Arming
    // without creating would therefore fail for the FIRST workflow of every
    // session and appear to work only from the second onwards.
    //
    // This writes into claude's own tree, which is why it needs saying:
    // claude creates the identical directory milliseconds later, so this
    // races nothing, and if no workflow ever runs it leaves one empty inert
    // directory behind.
    //
    // Sufficient against all three orderings:
    //   * armed before `wf_*` is born — the recursive watch sees the
    //     `Create` and notify arms the new subdirectory itself;
    //   * armed after the tree exists — `watch(Recursive)` arms existing
    //     subdirectories at call time, and the caller's first `notify_one`
    //     does the first read;
    //   * armed in between, losing `journal.jsonl`'s create — harmless:
    //     `read_tick` reads from the stored offset to EOF on ANY signal, and
    //     the next append to a 17-minute run's journal follows within
    //     seconds. The design is self-healing against a lost create.
    std::fs::create_dir_all(workflows_dir).map_err(CoreError::Io)?;

    let notify = Arc::new(Notify::new());
    let signal = notify.clone();
    let mut debouncer = new_debouncer(
        Duration::from_millis(300),
        None,
        move |result: DebounceEventResult| {
            if let Ok(events) = result {
                handle_batch(&events, &signal, on_change.as_ref());
            }
        },
    )
    .map_err(notify_to_core)?;
    debouncer
        .watch(workflows_dir, RecursiveMode::Recursive)
        .map_err(notify_to_core)?;

    Ok((
        WorkflowWatch {
            _debouncer: debouncer,
            reader: None,
        },
        notify,
    ))
}

/// The debouncer callback's body, lifted out of the closure so the
/// [`batch_signals_change`] guard can be exercised at its real call site.
///
/// Keeping it inline made the guard untestable without a live watcher: the
/// only test that reached it had to drive a real filesystem, so it carries
/// `#[ignore]` for timing and does not run in a default `cargo test`. A test
/// that calls [`batch_signals_change`] directly is no substitute — it stays
/// green if this function stops consulting it, which is exactly the
/// regression that matters. Taking `events` by slice makes the guard
/// reachable from a plain unit test with synthetic events, no filesystem and
/// no debouncer involved, so a default run catches its removal.
fn handle_batch(events: &[DebouncedEvent], signal: &Notify, on_change: &(dyn Fn() + Send + Sync)) {
    if batch_signals_change(events) {
        signal.notify_one();
        on_change();
    }
}

/// Whether a debounced batch mutated the run tree, or is only this reader's
/// own reads coming back at it.
///
/// The `IN_OPEN` hazard, structurally identical to the one
/// [`signals_change`] was written for: notify's inotify mask includes
/// `IN_OPEN`, so on Linux *reading* a watched file reports `Access(Open)` —
/// and this watcher reads every file it watches. Unfiltered, one
/// `read_tick` fires the watch, which schedules another `read_tick`, forever,
/// at real CPU cost with the app idle. This codebase has already shipped
/// that loop once (the git-status/`.git/index` spin `signals_change`
/// documents), so the filter is reused rather than re-derived.
fn batch_signals_change(events: &[DebouncedEvent]) -> bool {
    events.iter().any(|event| signals_change(&event.kind))
}

/// Read whatever is newly available in `path` since its stored offset (0 if
/// unseen), advancing the offset only past complete lines — a trailing
/// partial line (the file is actively being appended to) stays unconsumed
/// for the next tick, and the offset map gains no entry when nothing was
/// consumed, so "has this file ever had a complete line read from it" stays
/// answerable from map membership alone. `None` when the file doesn't exist
/// yet (a journal `started` line can land before the CLI creates the
/// agent's own file).
fn read_new_lines(path: &Path, offsets: &mut HashMap<PathBuf, u64>) -> Option<Vec<String>> {
    let mut file = File::open(path).ok()?;
    let start = offsets.get(path).copied().unwrap_or(0);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    let Some(last_newline) = buf.iter().rposition(|&b| b == b'\n') else {
        return Some(Vec::new());
    };
    let consumed = last_newline as u64 + 1;
    let text = String::from_utf8_lossy(&buf[..consumed as usize]);
    let lines = text
        .lines()
        .map(str::to_string)
        .filter(|l| !l.trim().is_empty())
        .collect();
    offsets.insert(path.to_path_buf(), start + consumed);
    Some(lines)
}

/// One line of an `agent-<id>.jsonl` to session events, mirroring the main
/// stdout pipeline (`protocol::parse_line` → stamp `parent_id` →
/// `driver::parsed_to_events`) behind two drops: the agent's injected
/// prompt (identified by the caller, see [`is_prompt_line`]) and
/// `attachment` lines.
///
/// Deliberately does NOT apply the keep-filter — that is
/// [`keep_new_tool_events`]'s job. Split so the two drops above are
/// assertable: the keep-filter discards `ProtocolError`, `Usage` and
/// `AssistantText` anyway, so a test against the filtered output would pass
/// with both of them deleted.
fn parse_agent_line(line: &str, parent_id: &str, is_prompt_line: bool) -> Vec<SessionEvent> {
    if is_prompt_line || is_attachment_line(line) {
        return Vec::new();
    }
    parsed_to_events(stamp_parent(protocol::parse_line(line), parent_id))
}

/// Narrow one agent line's events per "What We Are NOT Doing" in the plan:
/// only `ToolUse`/`ToolResult` survive — an agent's text and thinking are
/// dropped (the tool calls already answer "what is this agent doing," and
/// every tool block carries claude's own stable id, which is what makes
/// re-arm dedup exact at all) — and an id a re-arm already found in the
/// persisted log is skipped, keyed on the set matching its kind so a
/// persisted `ToolUse` can never suppress its own closing `ToolResult`.
fn keep_new_tool_events(
    events: Vec<SessionEvent>,
    seen_uses: &HashSet<String>,
    seen_results: &HashSet<String>,
) -> Vec<SessionEvent> {
    events
        .into_iter()
        .filter(|event| match event {
            SessionEvent::ToolUse { id, .. } => !seen_uses.contains(id),
            SessionEvent::ToolResult { tool_use_id, .. } => !seen_results.contains(tool_use_id),
            _ => false,
        })
        .collect()
}

/// `{"type":"attachment",...}` lines carry no `message` key at all
/// (confirmed against real workflow output), so routing one through
/// `parse_line` would degrade to `ParsedLine::Unknown` → a `ProtocolError`
/// event: noisy, and wrong, since this is a known, deliberately-unhandled
/// kind, not an actually-unrecognized line. Peeked before the general parse
/// rather than matched after, so a genuinely unknown line still errors the
/// same way it does everywhere else in the app.
fn is_attachment_line(line: &str) -> bool {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|v| v.get("type").and_then(Value::as_str).map(str::to_string))
        .is_some_and(|t| t == "attachment")
}

/// The agent's injected prompt is the one line in `agent-<id>.jsonl` with
/// `"parentUuid": null` — every other line names the uuid it answers. "The
/// first `type:"user"` line" would say the same thing but isn't decidable
/// from one line, and a positional rule would misfire the moment a tick's
/// batch starts mid-file.
fn is_prompt_line(line: &str) -> bool {
    serde_json::from_str::<Value>(line)
        .ok()
        .is_some_and(|v| matches!(v.get("parentUuid"), Some(Value::Null)))
}

/// Rewrite a parsed line's `parent_id` to the agent's synthetic tool id, so
/// `parsed_to_events` nests the resulting events under it and suppresses
/// `Usage` via its existing `parent_id.is_none()` gate (`driver.rs:676-678`)
/// — a file line has no `parent_tool_use_id` of its own, and replaying it
/// unstamped would move the main session's context meter. Only
/// `Assistant`/`User` carry a `parent_id`; every other variant passes
/// through unchanged (irrelevant here in practice, since every file line is
/// one of those two, but this keeps the function total).
fn stamp_parent(parsed: ParsedLine, parent_id: &str) -> ParsedLine {
    match parsed {
        ParsedLine::Assistant {
            blocks,
            context_tokens,
            ..
        } => ParsedLine::Assistant {
            blocks,
            context_tokens,
            parent_id: Some(parent_id.to_string()),
        },
        ParsedLine::User {
            blocks,
            uuid,
            is_replay,
            ..
        } => ParsedLine::User {
            blocks,
            parent_id: Some(parent_id.to_string()),
            uuid,
            is_replay,
        },
        other => other,
    }
}

/// The label for an agent's synthetic parent, read off its prompt line:
/// `{"type":"user",...,"message":{"role":"user","content":"<prompt>"}}`,
/// `content` a bare STRING (confirmed against real workflow output), read
/// directly here rather than via `parse_blocks` (`protocol.rs:380-386`,
/// which requires an array and returns nothing for a string) —
/// deliberately: were a future claude build to send an array of text blocks
/// on this line instead, `parse_blocks` would produce them and
/// `block_to_event` would map them to `AssistantText`, dumping the agent's
/// whole injected prompt into the transcript. Reading `message.content`
/// directly sidesteps that regardless of which shape arrives. `None` for
/// anything else — the caller falls back to the bare agent id.
fn agent_label_from_prompt_line(line: &str) -> Option<String> {
    let v: Value = serde_json::from_str(line).ok()?;
    let content = v.get("message")?.get("content")?.as_str()?;
    Some(agent_label(content))
}

/// **The first non-empty line after the prompt's first blank line**, falling
/// back to the first line when there is no blank.
///
/// Not the first line: a workflow prompt opens with a preamble the script
/// hands every agent, so a first-line rule collapses a run into rows that
/// all read alike — seven of the eight agents in the evidence run
/// (`wf_89a9bc27-4f1`) begin `Repo root: … Read-only.`, three of them
/// byte-identical, and the eight first lines yield only four distinct
/// strings. The line after the blank is the agent's actual assignment, and
/// yields eight distinct labels on that same run.
fn agent_label(prompt: &str) -> String {
    let prompt = prompt.trim();
    let mut lines = prompt.lines();
    let first = lines.next().unwrap_or("");
    let after_blank = lines
        .skip_while(|l| !l.trim().is_empty())
        .find(|l| !l.trim().is_empty());
    truncate_label(after_blank.unwrap_or(first))
}

/// A one-line label a UI can render beside a tree row, not the prompt
/// itself: trimmed and capped at [`LABEL_MAX_CHARS`] (mirrors the
/// frontend's `firstLine()`, `backgroundTask.ts`).
const LABEL_MAX_CHARS: usize = 200;

fn truncate_label(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= LABEL_MAX_CHARS {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(LABEL_MAX_CHARS).collect();
        format!("{truncated}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL_LAUNCH_RESULT: &str = "Workflow launched in background. Task ID: whjv5o0xq\nSummary: Map how hyprlayer-app drives spec-driven dev stages, and what each transition actually writes\nTranscript dir: /home/jt/.claude/projects/slug/session/subagents/workflows/wf_89a9bc27-4f1\nScript file: /home/jt/.claude/projects/slug/session/workflows/scripts/map-stage-driving-wf_89a9bc27-4f1.js\n(Edit this file with Write/Edit and re-invoke Workflow with {scriptPath: \"…\"} to iterate without resending the script.)\nRun ID: wf_89a9bc27-4f1\nTo resume after editing the script: Workflow({scriptPath: \"…\", resumeFromRunId: \"wf_89a9bc27-4f1\"}) — completed agents return cached results (…).\n\nYou will be notified when it completes. Use /workflows to watch live progress.";

    const PROMPT_LINE: &str = r#"{"parentUuid":null,"isSidechain":true,"agentId":"a1","type":"user","message":{"role":"user","content":"Repo root: /repo/checkout. Read-only.\n\nMap what happens when the app launches an agent session."},"uuid":"u1"}"#;
    const TOOL_USE_LINE: &str = r#"{"parentUuid":"u1","type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"ls"}}]}}"#;
    const TOOL_RESULT_LINE: &str = r#"{"parentUuid":"u2","type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"ok"}]}}"#;

    #[test]
    fn workflow_launch_from_result_parses_the_real_shape() {
        let launch = workflow_launch_from_result(REAL_LAUNCH_RESULT).expect("should parse");
        assert_eq!(
            launch.run_dir,
            PathBuf::from(
                "/home/jt/.claude/projects/slug/session/subagents/workflows/wf_89a9bc27-4f1"
            )
        );
        assert_eq!(launch.run_id, "wf_89a9bc27-4f1");
        assert_eq!(launch.task_id, "whjv5o0xq");
    }

    #[test]
    fn workflow_launch_from_result_returns_none_for_non_workflow_result() {
        let bash_output = "total 12\ndrwxr-xr-x  3 jt jt 4096 Aug 22 10:00 .\n-rw-r--r--  1 jt jt  123 Aug 22 10:00 Cargo.toml\n";
        assert!(workflow_launch_from_result(bash_output).is_none());
        assert!(workflow_launch_from_result("").is_none());
    }

    /// The `Summary:` line is model-authored and can wrap across several
    /// physical lines; a parser that assumed one field per fixed line
    /// position would misread `Transcript dir:`/`Run ID:` after it. Field
    /// prefixes don't care where in the block they land.
    #[test]
    fn workflow_launch_from_result_handles_a_wrapped_summary() {
        let wrapped = "Workflow launched in background. Task ID: t1\nSummary: This summary is long enough that the model\nwrapped it across three physical lines before finally\nfinishing the sentence here.\nTranscript dir: /a/b/wf_x1\nRun ID: wf_x1\n\nYou will be notified when it completes.";
        let launch = workflow_launch_from_result(wrapped).expect("should parse despite wrapping");
        assert_eq!(launch.run_dir, PathBuf::from("/a/b/wf_x1"));
        assert_eq!(launch.run_id, "wf_x1");
        assert_eq!(launch.task_id, "t1");
    }

    /// Disagreement between the parsed `Run ID:` and the transcript dir's own
    /// basename means the wire format moved under us; degrade to `None`
    /// rather than trust a pairing that doesn't cross-check.
    #[test]
    fn workflow_launch_from_result_returns_none_on_run_id_mismatch() {
        let mismatched = "Workflow launched in background. Task ID: t1\nTranscript dir: /a/b/wf_x1\nRun ID: wf_DIFFERENT\n";
        assert!(workflow_launch_from_result(mismatched).is_none());
    }

    #[test]
    fn workflow_launch_from_result_requires_every_field() {
        assert!(workflow_launch_from_result("Task ID: t1\nRun ID: wf_x1\n").is_none());
        assert!(workflow_launch_from_result("Task ID: t1\nTranscript dir: /a/wf_x1\n").is_none());
        assert!(workflow_launch_from_result("Transcript dir: /a/wf_x1\nRun ID: wf_x1\n").is_none());
    }

    /// Asserted against `parse_agent_line` rather than the keep-filtered
    /// output: the keep-filter drops `ProtocolError` too, so a test one
    /// layer down would pass with `is_attachment_line` deleted.
    #[test]
    fn parse_agent_line_drops_an_attachment_line() {
        let line = r#"{"parentUuid":"p1","isSidechain":true,"agentId":"a1","attachment":{"type":"skill_listing","names":["a","b"]},"type":"attachment","uuid":"u1"}"#;
        assert!(parse_agent_line(line, "wf:run:a1", false).is_empty());
    }

    /// The counterpart to the test above: a genuinely unrecognized line is
    /// NOT swallowed by the attachment peek, it still reaches
    /// `ProtocolError` the way it does everywhere else in the app. Only the
    /// keep-filter drops it.
    #[test]
    fn parse_agent_line_still_reports_a_genuinely_unknown_line() {
        let line = r#"{"type":"brand_new_event_kind","x":1}"#;
        let parsed = parse_agent_line(line, "wf:run:a1", false);
        assert!(
            matches!(parsed.as_slice(), [SessionEvent::ProtocolError { .. }]),
            "expected a ProtocolError, got {parsed:?}"
        );
        assert!(keep_new_tool_events(parsed, &HashSet::new(), &HashSet::new()).is_empty());
    }

    /// The agent's own injected prompt must never render in the transcript,
    /// regardless of whether `content` is the bare string real data uses or
    /// a hypothetical future array of text blocks. Asserted against
    /// `parse_agent_line`: today the array shape is the only one that would
    /// produce anything, and the keep-filter would hide the difference.
    #[test]
    fn parse_agent_line_drops_the_prompt_line_whatever_its_content_shape() {
        let array_of_blocks = r#"{"parentUuid":null,"type":"user","message":{"role":"user","content":[{"type":"text","text":"Repo root: /repo. Read-only."}]}}"#;
        for line in [PROMPT_LINE, array_of_blocks] {
            assert!(
                parse_agent_line(line, "wf:run:a1", true).is_empty(),
                "a prompt line must yield no events: {line}"
            );
        }
        // Without the flag, the array shape would have reached the transcript.
        assert!(!parse_agent_line(array_of_blocks, "wf:run:a1", false).is_empty());
    }

    /// Every event carries the agent's synthetic id, which is what nests it,
    /// and a live usage ping is suppressed by `driver.rs:676-678`'s
    /// `parent_id.is_none()` gate — replayed unstamped it would move the
    /// MAIN session's context meter. Asserted before the keep-filter, which
    /// drops `Usage` and `AssistantText` on its own.
    #[test]
    fn parse_agent_line_stamps_parent_id_and_emits_no_usage() {
        let assistant = r#"{"parentUuid":"u1","type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"thinking out loud"},{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"ls"}}],"usage":{"input_tokens":100}}}"#;
        let events = parse_agent_line(assistant, "wf:run:a1", false);
        assert_eq!(events.len(), 2, "text + tool_use, no usage: {events:?}");
        for event in &events {
            let parent_id = match event {
                SessionEvent::AssistantText { parent_id, .. } => parent_id,
                SessionEvent::ToolUse { parent_id, .. } => parent_id,
                other => panic!("unexpected event kind: {other:?}"),
            };
            assert_eq!(parent_id.as_deref(), Some("wf:run:a1"));
        }
    }

    #[test]
    fn is_prompt_line_matches_only_an_explicitly_null_parent_uuid() {
        assert!(is_prompt_line(PROMPT_LINE));
        assert!(!is_prompt_line(TOOL_USE_LINE));
        // No `parentUuid` key at all is not the prompt line either — that is
        // what a journal line looks like.
        assert!(!is_prompt_line(
            r#"{"type":"started","key":"k","agentId":"a1"}"#
        ));
        assert!(!is_prompt_line("not json"));
    }

    #[test]
    fn agent_label_takes_the_line_after_the_first_blank() {
        assert_eq!(
            agent_label(
                "Repo root: /repo/checkout. Read-only.\n\nMap the STAGE MODEL of the pipeline."
            ),
            "Map the STAGE MODEL of the pipeline."
        );
        // Blank runs and leading whitespace don't shift the answer.
        assert_eq!(
            agent_label("  Preamble.\n   \n\n  The assignment.\nMore detail."),
            "The assignment."
        );
        // No blank line anywhere: the first line is all there is.
        assert_eq!(
            agent_label("One long single-line prompt."),
            "One long single-line prompt."
        );
        // A trailing blank with nothing after it falls back the same way.
        assert_eq!(agent_label("Only a preamble.\n\n"), "Only a preamble.");
    }

    #[test]
    fn agent_label_from_prompt_line_is_none_for_non_string_content() {
        let array_content =
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}"#;
        assert!(agent_label_from_prompt_line(array_content).is_none());
        assert!(agent_label_from_prompt_line("not json").is_none());
    }

    #[test]
    fn truncate_label_caps_a_long_line() {
        let long = "x".repeat(400);
        let out = truncate_label(&long);
        assert_eq!(out.chars().count(), LABEL_MAX_CHARS + 1); // +1 for the ellipsis
        assert!(out.ends_with('…'));
        assert_eq!(truncate_label("  padded  "), "padded");
    }

    /// A run whose files sit under `dir`: one journal line per entry in
    /// `journal`, one agent file per `(agent_id, lines)` pair.
    fn write_run(dir: &Path, journal: &[&str], agents: &[(&str, &[&str])]) {
        std::fs::create_dir_all(dir).unwrap();
        let mut journal_text = String::new();
        for line in journal {
            journal_text.push_str(line);
            journal_text.push('\n');
        }
        std::fs::write(dir.join("journal.jsonl"), journal_text).unwrap();
        for (agent_id, lines) in agents {
            let mut text = String::new();
            for line in *lines {
                text.push_str(line);
                text.push('\n');
            }
            std::fs::write(dir.join(format!("agent-{agent_id}.jsonl")), text).unwrap();
        }
    }

    fn append(path: &Path, text: &str) {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        f.write_all(text.as_bytes()).unwrap();
    }

    fn tool_use_line(id: &str) -> String {
        format!(
            r#"{{"parentUuid":"u1","type":"assistant","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"{id}","name":"Bash","input":{{"command":"ls"}}}}]}}}}"#
        )
    }

    /// Every emitted event's id, in emission order — synthetic parents and
    /// the agents' own tool calls alike.
    fn ids(events: &[SessionEvent]) -> Vec<&str> {
        events
            .iter()
            .map(|e| match e {
                SessionEvent::ToolUse { id, .. } => id.as_str(),
                SessionEvent::ToolResult { tool_use_id, .. } => tool_use_id.as_str(),
                other => panic!("unexpected event kind: {other:?}"),
            })
            .collect()
    }

    fn launch(run_dir: PathBuf, run_id: &str) -> WorkflowLaunch {
        WorkflowLaunch {
            run_dir,
            run_id: run_id.to_string(),
            task_id: "t1".into(),
        }
    }

    fn registered(run_dir: PathBuf, run_id: &str) -> WorkflowRuns {
        let mut runs = WorkflowRuns::new();
        runs.register(
            WorkflowLaunch {
                run_dir,
                run_id: run_id.to_string(),
                task_id: "t1".into(),
            },
            "toolu_workflow".into(),
            7,
        );
        runs
    }

    /// `journal.jsonl` is read before any agent file, so in a window where an
    /// already-open agent appended AND a new one started, the new agent's
    /// synthetic parent lands ahead of the older agent's new content. Nesting
    /// is order-independent (`childrenByParent`), but a child whose
    /// `parentId` names an item not yet in the log is filtered out of
    /// `topLevel` and stays invisible until the parent lands.
    #[test]
    fn a_tick_reads_the_journal_before_the_agent_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let run_dir = tmp.path().join("wf_t");
        write_run(
            &run_dir,
            &[r#"{"type":"started","key":"k1","agentId":"a1"}"#],
            &[("a1", &[PROMPT_LINE, &tool_use_line("toolu_1")])],
        );
        let mut runs = registered(run_dir.clone(), "wf_t");
        assert_eq!(ids(&runs.read_tick()), ["wf:wf_t:a1", "toolu_1"]);

        append(
            &run_dir.join("agent-a1.jsonl"),
            &format!("{}\n", tool_use_line("toolu_2")),
        );
        std::fs::write(
            run_dir.join("agent-a2.jsonl"),
            format!("{PROMPT_LINE}\n{}\n", tool_use_line("toolu_3")),
        )
        .unwrap();
        append(
            &run_dir.join("journal.jsonl"),
            "{\"type\":\"started\",\"key\":\"k2\",\"agentId\":\"a2\"}\n",
        );

        assert_eq!(
            ids(&runs.read_tick()),
            ["wf:wf_t:a2", "toolu_3", "toolu_2"],
            "the journal's new agent opens before an older agent's new content"
        );
    }

    /// Why `seen_uses` and `seen_results` are two sets and not one: a
    /// `ToolUse` persisted before a restart is in the seeded set, and its
    /// `ToolResult` is the exact same string (`events.rs:229-241`). One set
    /// suppresses both, leaving every such row — the synthetic per-agent
    /// parents included — open forever. Split, each half is skipped only if
    /// that half was actually persisted.
    #[test]
    fn a_seen_tool_use_does_not_suppress_its_own_tool_result() {
        let tmp = tempfile::TempDir::new().unwrap();
        let run_dir = tmp.path().join("wf_t");
        write_run(
            &run_dir,
            &[
                r#"{"type":"started","key":"k1","agentId":"a1"}"#,
                r#"{"type":"result","key":"k1","agentId":"a1","result":"done"}"#,
            ],
            &[("a1", &[PROMPT_LINE, TOOL_USE_LINE, TOOL_RESULT_LINE])],
        );
        let mut runs = registered(run_dir, "wf_t");

        // Seed exactly what a re-arm would find in the persisted log: both
        // the real tool's use and the synthetic parent's, neither result.
        let run = runs.runs.get_mut("wf_t").unwrap();
        run.seen_uses.insert("toolu_1".into());
        run.seen_uses.insert("wf:wf_t:a1".into());

        let events = runs.read_tick();
        let uses: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                SessionEvent::ToolUse { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect();
        let results: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                SessionEvent::ToolResult { tool_use_id, .. } => Some(tool_use_id.as_str()),
                _ => None,
            })
            .collect();

        assert!(
            uses.is_empty(),
            "both ToolUses were already persisted: {uses:?}"
        );
        assert!(
            results.contains(&"toolu_1"),
            "the agent's own ToolResult must still close its row: {results:?}"
        );
        assert!(
            results.contains(&"wf:wf_t:a1"),
            "the synthetic parent's ToolResult must still close its row: {results:?}"
        );
    }

    /// A rewind re-registers runs it did not cut while the reader task — and
    /// its advanced offsets — survive (Phase 4 §7/§8), so a `register` that
    /// reset the cursors would re-read the whole run from byte 0.
    #[test]
    fn register_on_an_already_present_run_is_a_no_op() {
        let tmp = tempfile::TempDir::new().unwrap();
        let run_dir = tmp.path().join("wf_t");
        write_run(
            &run_dir,
            &[r#"{"type":"started","key":"k1","agentId":"a1"}"#],
            &[("a1", &[PROMPT_LINE, TOOL_USE_LINE])],
        );
        let mut runs = registered(run_dir.clone(), "wf_t");
        assert!(!runs.read_tick().is_empty());
        let offsets = runs.runs["wf_t"].offsets.clone();
        assert!(!offsets.is_empty(), "the first tick advanced the cursors");

        runs.register(
            WorkflowLaunch {
                run_dir,
                run_id: "wf_t".into(),
                task_id: "t2".into(),
            },
            "toolu_other".into(),
            99,
        );

        let run = &runs.runs["wf_t"];
        assert_eq!(run.offsets, offsets, "cursors must survive a re-register");
        assert_eq!(run.workflow_tool_use_id, "toolu_workflow");
        assert_eq!(run.armed_seq, 7);
        assert!(
            runs.read_tick().is_empty(),
            "a re-register must not replay what the surviving cursors already covered"
        );
    }

    /// Nothing guarantees the CLI has created the agent's file when its
    /// journal `started` line lands, and a placeholder label can't be
    /// corrected once persisted — so the open is deferred, and the file's
    /// own creation fires the watcher that schedules the retry.
    #[test]
    fn a_deferred_open_is_retried_once_the_agent_file_appears() {
        let tmp = tempfile::TempDir::new().unwrap();
        let run_dir = tmp.path().join("wf_t");
        write_run(
            &run_dir,
            &[r#"{"type":"started","key":"k1","agentId":"a1"}"#],
            &[],
        );
        let mut runs = registered(run_dir.clone(), "wf_t");
        assert!(
            runs.read_tick().is_empty(),
            "no agent file yet: nothing to label, so nothing to emit"
        );

        std::fs::write(
            run_dir.join("agent-a1.jsonl"),
            format!("{PROMPT_LINE}\n{TOOL_USE_LINE}\n"),
        )
        .unwrap();

        let events = runs.read_tick();
        match events.first() {
            Some(SessionEvent::ToolUse {
                id, name, input, ..
            }) => {
                assert_eq!(id, "wf:wf_t:a1");
                assert_eq!(name, "Agent");
                assert!(
                    input.contains("Map what happens when the app launches an agent session."),
                    "the deferred open carries the real label, not the bare agent id: {input}"
                );
            }
            other => panic!("expected the synthetic parent ToolUse first, got {other:?}"),
        }
        assert!(events
            .iter()
            .any(|e| matches!(e, SessionEvent::ToolUse { id, .. } if id == "toolu_1")));
    }

    /// The reader's own cursor: an agent file appended to mid-read (the
    /// normal case — the CLI writes it while we tail it) must yield the
    /// SAME event sequence whether it's read complete in one tick or split
    /// into two ticks with the second chunk completing a line the first
    /// tick left partial.
    #[test]
    fn chunked_agent_file_append_matches_a_single_read() {
        fn run(root: &Path, writes: &[&str]) -> Vec<SessionEvent> {
            let run_dir = root.join("wf_t");
            write_run(
                &run_dir,
                &[r#"{"type":"started","key":"k1","agentId":"a1"}"#],
                &[("a1", &[])],
            );
            let agent_path = run_dir.join("agent-a1.jsonl");
            let mut runs = registered(run_dir, "wf_t");

            let mut events = Vec::new();
            for chunk in writes {
                use std::io::Write;
                let mut f = std::fs::OpenOptions::new()
                    .append(true)
                    .open(&agent_path)
                    .unwrap();
                f.write_all(chunk.as_bytes()).unwrap();
                drop(f);
                events.extend(runs.read_tick());
            }
            events
        }

        let full = format!("{PROMPT_LINE}\n{TOOL_USE_LINE}\n");

        let single_dir = tempfile::TempDir::new().unwrap();
        let single = run(single_dir.path(), &[&full]);

        // Split mid-way through the second (tool_use) line: the first write
        // leaves a partial trailing line, which the cursor must not consume.
        let split_at = PROMPT_LINE.len() + 1 + TOOL_USE_LINE.len() / 2;
        let (first, rest) = full.split_at(split_at);
        let split_dir = tempfile::TempDir::new().unwrap();
        let split = run(split_dir.path(), &[first, rest]);

        assert_eq!(single.len(), 2, "synthetic parent + the agent's tool_use");
        assert_eq!(
            serde_json::to_string_pretty(&single).unwrap(),
            serde_json::to_string_pretty(&split).unwrap(),
            "chunked read must match a single read"
        );
    }

    /// The journal's `result` string is emitted verbatim, like every other
    /// tool result in the app (`protocol.rs`'s `tool_result_text` has no cap
    /// either) — `ToolResultOutput` already scrolls a long one.
    #[test]
    fn a_long_journal_result_is_emitted_verbatim() {
        let tmp = tempfile::TempDir::new().unwrap();
        let run_dir = tmp.path().join("wf_t");
        let long = "x".repeat(30_000);
        write_run(
            &run_dir,
            &[
                r#"{"type":"started","key":"k1","agentId":"a1"}"#,
                &format!(r#"{{"type":"result","key":"k1","agentId":"a1","result":"{long}"}}"#),
            ],
            &[("a1", &[PROMPT_LINE])],
        );
        let events = registered(run_dir, "wf_t").read_tick();
        let closed = events
            .iter()
            .find_map(|e| match e {
                SessionEvent::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } if tool_use_id == "wf:wf_t:a1" => Some(content),
                _ => None,
            })
            .expect("the agent's synthetic parent should close");
        assert_eq!(closed, &long);
    }

    /// A debounced batch of pure reads must not count as a change. This
    /// watcher reads the very files it watches, so an unfiltered `IN_OPEN`
    /// (Linux inotify reports one per open) makes every `read_tick` schedule
    /// the next — the CPU spin `signals_change` exists for, at a new seam.
    ///
    /// Asserted against `batch_signals_change`, the workflow watcher's OWN
    /// filter, not against `signals_change` directly: `signals_change` is
    /// reused unchanged, so a test one layer down would still pass with the
    /// call deleted from this path.
    #[test]
    fn a_batch_of_reads_does_not_signal_a_workflow_change() {
        use notify::event::{AccessKind, AccessMode, CreateKind, DataChange, ModifyKind};
        use notify::{Event, EventKind};

        let journal = PathBuf::from("/s/subagents/workflows/wf_x/journal.jsonl");
        let at = std::time::Instant::now();
        let batch = |kinds: &[EventKind]| -> Vec<DebouncedEvent> {
            kinds
                .iter()
                .map(|kind| DebouncedEvent::new(Event::new(*kind).add_path(journal.clone()), at))
                .collect()
        };

        let reads = [
            EventKind::Access(AccessKind::Open(AccessMode::Read)),
            EventKind::Access(AccessKind::Open(AccessMode::Any)),
            EventKind::Access(AccessKind::Read),
            EventKind::Access(AccessKind::Close(AccessMode::Read)),
        ];
        assert!(
            !batch_signals_change(&batch(&reads)),
            "a whole batch of reads is this reader's own tail coming back"
        );

        // The appends the run actually makes still get through, and one of
        // them among any number of reads is enough.
        let append = EventKind::Modify(ModifyKind::Data(DataChange::Any));
        assert!(batch_signals_change(&batch(&[append])));
        assert!(batch_signals_change(&batch(&[EventKind::Create(
            CreateKind::File
        )])));
        assert!(batch_signals_change(&batch(&[
            reads[0], reads[1], append, reads[2]
        ])));
        assert!(!batch_signals_change(&batch(&[])));
    }

    /// The same guard, at its real call site.
    ///
    /// [`a_batch_of_reads_does_not_signal_a_workflow_change`] pins the filter
    /// itself and stays green if [`handle_batch`] stops consulting it, so it
    /// cannot catch the deletion that actually matters. The one test that did
    /// reach the call site needed a live watcher, so it carries `#[ignore]`
    /// for fs timing and does not run in a default `cargo test` — leaving the
    /// line unguarded in every run anyone makes. This drives [`handle_batch`]
    /// directly with synthetic batches: no filesystem, no debouncer, no
    /// timing, so it runs by default.
    #[test]
    fn the_callback_ignores_a_read_only_batch_at_its_call_site() {
        use notify::event::{AccessKind, AccessMode, DataChange, ModifyKind};
        use notify::{Event, EventKind};
        use std::sync::atomic::{AtomicUsize, Ordering};

        let journal = PathBuf::from("/s/subagents/workflows/wf_x/journal.jsonl");
        let at = std::time::Instant::now();
        let batch = |kinds: &[EventKind]| -> Vec<DebouncedEvent> {
            kinds
                .iter()
                .map(|kind| DebouncedEvent::new(Event::new(*kind).add_path(journal.clone()), at))
                .collect()
        };

        let signal = Notify::new();
        let fired = Arc::new(AtomicUsize::new(0));
        let counter = fired.clone();
        let on_change: Box<dyn Fn() + Send + Sync> = Box::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });

        let reads = batch(&[
            EventKind::Access(AccessKind::Open(AccessMode::Read)),
            EventKind::Access(AccessKind::Close(AccessMode::Read)),
        ]);
        handle_batch(&reads, &signal, on_change.as_ref());
        assert_eq!(
            fired.load(Ordering::SeqCst),
            0,
            "this reader's own reads must not reach on_change — that is the CPU spin"
        );

        // Paired with the negative case so the test cannot pass by never
        // firing at all, which is how a guard test quietly goes vacuous.
        let append = batch(&[EventKind::Modify(ModifyKind::Data(DataChange::Any))]);
        handle_batch(&append, &signal, on_change.as_ref());
        assert_eq!(
            fired.load(Ordering::SeqCst),
            1,
            "a real append must still reach on_change"
        );
    }

    /// The run tree does not exist when the launch tool_result lands
    /// (`subagents/workflows/` is born +8 to +45 ms later), and
    /// `debouncer.watch()` on a missing path is an error — so arming without
    /// creating fails for the FIRST workflow of every session and looks fine
    /// from the second onwards.
    #[test]
    fn watch_workflows_creates_the_directory_it_arms() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("session/subagents/workflows");
        assert!(!dir.exists(), "the fixture starts with nothing on disk");

        let (watch, _notify) =
            watch_workflows(&dir, Box::new(|| {})).expect("arming must not fail on a missing dir");
        assert!(dir.is_dir(), "watch_workflows creates what it arms");
        drop(watch);
    }

    /// One watch per session, not per run: the second workflow of a session
    /// joins the first one's watch and its reader task. Registering it still
    /// adds it to the map notify's own subdirectory arming will cover.
    #[test]
    fn a_second_run_joins_the_sessions_one_watch() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workflows_dir = tmp.path().join("subagents/workflows");
        let mut session = SessionWorkflows::new();
        assert!(!session.is_armed());

        assert!(session.runs.register_seeded(
            launch(workflows_dir.join("wf_a"), "wf_a"),
            "toolu_a".into(),
            3,
            HashSet::new(),
            HashSet::new(),
        ));
        let (first, armed) = session.arm(&workflows_dir, Box::new(|| {})).unwrap();
        assert!(armed, "the first launch arms the session's watch");
        assert!(session.is_armed());

        assert!(session.runs.register_seeded(
            launch(workflows_dir.join("wf_b"), "wf_b"),
            "toolu_b".into(),
            9,
            HashSet::new(),
            HashSet::new(),
        ));
        let (second, armed_again) = session.arm(&workflows_dir, Box::new(|| {})).unwrap();
        assert!(!armed_again, "the second run must not arm a second watcher");
        assert!(
            Arc::ptr_eq(&first, &second),
            "and it must wake the reader the first one already started"
        );
        assert_eq!(session.runs.len(), 2, "both runs are being tailed");
    }

    /// A live rewind cuts `seq >= cutoff`, so the runs whose arming
    /// `ToolResult` survived the cut are exactly those with
    /// `armed_seq < cutoff`. Keeping a cut run would leave its reader
    /// injecting children of a parent row that no longer exists; dropping an
    /// uncut one would strand a live run mid-flight.
    #[test]
    fn a_rewind_drops_only_the_runs_it_cut() {
        let mut runs = WorkflowRuns::new();
        runs.register(
            launch(PathBuf::from("/a/wf_below"), "wf_below"),
            "t1".into(),
            4,
        );
        runs.register(launch(PathBuf::from("/a/wf_at"), "wf_at"), "t2".into(), 7);
        runs.register(
            launch(PathBuf::from("/a/wf_above"), "wf_above"),
            "t3".into(),
            9,
        );

        runs.retain_armed_below(7);

        assert_eq!(runs.len(), 1);
        assert!(runs.runs.contains_key("wf_below"));
        // `cut_session_events_from` deletes `seq >= cutoff`, so the run armed
        // exactly AT the cutoff lost its parent row too.
        assert!(!runs.runs.contains_key("wf_at"));
        assert!(!runs.runs.contains_key("wf_above"));
    }

    // The `IN_OPEN` guard through the REAL call site, not the pure filter.
    // `a_batch_of_reads_does_not_signal_a_workflow_change` above calls
    // `batch_signals_change` directly, so it stays green even if the `if
    // batch_signals_change(&events)` check inside `watch_workflows`'s
    // debouncer closure is deleted — the filter would simply go unused. This
    // test instead does exactly what the reader task does (open the watched
    // file and read it to EOF) against a real notify watch, and asserts the
    // callback does NOT fire from that read alone. Mirrors
    // `watcher.rs`'s `reading_the_git_index_does_not_fire`, the same test
    // shape for the worktree watcher's identical hazard. Ignored for the same
    // reason as the sibling test below: fs timing shouldn't gate releases.
    #[test]
    #[ignore = "fs-watcher timing; run explicitly with --ignored"]
    fn reading_a_watched_file_does_not_fire_the_callback() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let tmp = tempfile::tempdir().unwrap();
        let workflows_dir = tmp.path().join("session/subagents/workflows");
        let run_dir = workflows_dir.join("wf_existing");
        std::fs::create_dir_all(&run_dir).unwrap();
        let journal = run_dir.join("journal.jsonl");
        std::fs::write(
            &journal,
            "{\"type\":\"started\",\"key\":\"k1\",\"agentId\":\"a1\"}\n",
        )
        .unwrap();

        let hits = Arc::new(AtomicUsize::new(0));
        let seen = hits.clone();
        let (watch, _notify) = watch_workflows(
            &workflows_dir,
            Box::new(move || {
                seen.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .unwrap();
        // Let the watch arm and any create-triggered batch settle before
        // establishing the baseline.
        std::thread::sleep(Duration::from_millis(400));
        let baseline = hits.load(Ordering::SeqCst);

        // Exactly what `read_new_lines` does on every tick: open the file the
        // watch covers and read it to EOF.
        for _ in 0..5 {
            let mut file = File::open(&journal).unwrap();
            let mut buf = Vec::new();
            file.read_to_end(&mut buf).unwrap();
        }

        // Give the debouncer its full window to batch and fire if it's going to.
        std::thread::sleep(Duration::from_millis(600));
        drop(watch);
        assert_eq!(
            hits.load(Ordering::SeqCst),
            baseline,
            "reading the watched file must not fire on_change on its own"
        );
    }

    // Real notify watcher: the first workflow of a session arms BEFORE its
    // `wf_*` directory is born, so the recursive watch has to pick up a
    // subdirectory notify armed for itself and fire on the first journal
    // append inside it. Ignored: fs timing shouldn't gate releases
    // (`a_batch_of_reads_does_not_signal_a_workflow_change` covers the filter
    // deterministically, and it is the real gate).
    #[test]
    #[ignore = "fs-watcher timing; run explicitly with --ignored"]
    fn an_append_under_a_new_run_directory_fires_the_callback() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let tmp = tempfile::tempdir().unwrap();
        let workflows_dir = tmp.path().join("session/subagents/workflows");

        let hits = Arc::new(AtomicUsize::new(0));
        let seen = hits.clone();
        let (watch, _notify) = watch_workflows(
            &workflows_dir,
            Box::new(move || {
                seen.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .unwrap();
        // Let the OS arm the watch before mutating.
        std::thread::sleep(Duration::from_millis(400));

        // The launch's exact shape: the run directory appears after arming,
        // then the CLI starts appending to a journal inside it.
        let run_dir = workflows_dir.join("wf_new");
        std::fs::create_dir_all(&run_dir).unwrap();
        std::fs::write(
            run_dir.join("journal.jsonl"),
            "{\"type\":\"started\",\"key\":\"k1\",\"agentId\":\"a1\"}\n",
        )
        .unwrap();

        let fired = (0..50).any(|_| {
            if hits.load(Ordering::SeqCst) > 0 {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
            false
        });
        drop(watch);
        assert!(
            fired,
            "an append under a newly created wf_* must fire on_change"
        );
    }
}
