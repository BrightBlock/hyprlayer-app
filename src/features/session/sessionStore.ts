/**
 * Live agent-session state, keyed by backend session id. The store outlives any
 * one component: a session's transcript survives switching tabs/workstreams or
 * navigating to the Library and back, because the Tauri `Channel` that feeds it
 * is held here (module scope), not in a component.
 *
 * Flow:
 *  - `startSession` opens a Channel, calls `start_stage_session`, and pumps
 *    every `SessionEvent` through the pure `reduceEvent`. Events that arrive
 *    before the command resolves (and we learn the id) are buffered, then
 *    flushed in order behind the optimistic first user message.
 *  - `attachIfNeeded` re-binds a Channel for a session we have a tab for but no
 *    in-memory transcript (e.g. after an HMR reload): `attach_session` replays
 *    the SQLite log then tails the live stream — only ever into an ABSENT entry,
 *    so replay can't duplicate an existing transcript.
 *
 * The store is a plain `{ sessions }` bag; all mutations are module functions
 * (the common zustand pattern for side-effecting async actions).
 */

import { useMemo } from "react";
import { Channel } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { create } from "zustand";
import { useShallow } from "zustand/shallow";

import { commands, type Session, type SessionEvent } from "../../lib/bindings";
import { unwrap } from "../../lib/ipc";
import { queryClient } from "../../lib/queryClient";
import { invalidateWorktreeGit } from "../../lib/queries";
import { findWorkstreamForNotify, notifyNeedsAttention } from "../../lib/notifications";
import { router } from "../../router";
import { docFromTool, docTitle, type DocKind } from "./toolDisplay";
import {
  backgroundTasksFromItems,
  type BackgroundTask,
  type BackgroundTaskStatus,
} from "./backgroundTask";
import { type Attachment } from "./attachments";
import { composeStagePrompt } from "./stages";
import { openCommitTab, openShipTab, useWorkstreamTabsStore } from "./workstreamTabs";
import {
  addUserMessage,
  cancelPendingPrompts,
  hasPendingPermission,
  initialSessionView,
  isBusy,
  isTerminal,
  isWorking,
  parkedState,
  reduceEvent,
  resolvePermission,
  resolveQuestion,
  restoredView,
  viewToText,
  type SessionView,
} from "./transcript";

export type SessionEntry = SessionView & {
  workstreamId: string;
  stage: string;
  /** Tool names the user chose to auto-approve for the rest of this session. */
  autoAllow: string[];
  /** Whether a live claude process is attached. False for a session restored
   *  from the log after a restart — it resumes on the next message. */
  live: boolean;
  /** The `--permission-mode` this session launched with (null = stage default).
   *  Remembered so a resume re-applies it — a running session can't switch into
   *  `bypassPermissions`, so changing the mode restarts (kill + resume) with it. */
  permissionMode: string | null;
  /** True only during a permission-mode restart: the kill→resume swap. Lets the
   *  kill's `exited` event be swallowed so the transcript doesn't flash "ended". */
  reloading?: boolean;
  /** Messages the user sent while a turn was still in flight, awaiting delivery.
   *  Delivered one-per-turn as each turn finishes (conversation steering). Each
   *  carries its own staged attachments. */
  queued: QueuedMessage[];
  /** Epoch ms when the current working stretch began, or null when idle —
   *  drives the live "time working" counter. Maintained outside the pure
   *  reducer (it needs a clock) by `setEntry`, so every mutation keeps it right. */
  workingSince: number | null;
  /** Epoch ms when each background task (keyed by `toolUseId`) was first
   *  observed — drives the bar's per-task elapsed timer. Rebuilt from the live
   *  task set on every `setEntry`, so a task's stamp drops as soon as it ends
   *  (TaskStop or a completed poll). It is NOT cleared by session exit on its
   *  own — an un-stopped spawn stays in `items` — so the bar is kept from
   *  showing phantom tasks one layer up: a terminal session renders the "ended"
   *  banner instead of the composer, and `useSessionBackgroundTasks` guards on
   *  `isTerminal`. In-memory only; never persisted. Maintained outside the pure
   *  reducer like `workingSince`. */
  taskStartedAt: Record<string, number>;
};

/** A follow-up turn the user sent mid-turn, held until the current turn ends. */
export type QueuedMessage = { text: string; attachments: Attachment[] };

type State = { sessions: Record<string, SessionEntry> };

export const useSessionStore = create<State>(() => ({ sessions: {} }));

/** Live channels, kept alive past component unmount; dropped on exit. */
const channels = new Map<string, Channel<SessionEvent>>();

/** Sessions whose transcript is mid-restore from the SQLite log (dedupe). */
const restoring = new Set<string>();

function entryOf(sessionId: string): SessionEntry | undefined {
  return useSessionStore.getState().sessions[sessionId];
}

function setEntry(sessionId: string, next: SessionEntry): void {
  useSessionStore.setState((s) => {
    const prev = s.sessions[sessionId];
    const workingSince = nextWorkingSince(prev, next);
    const taskStartedAt = nextTaskStartedAt(prev, next);
    return { sessions: { ...s.sessions, [sessionId]: { ...next, workingSince, taskStartedAt } } };
  });
}

/**
 * Drop the in-memory transcripts for a workstream's sessions — called when the
 * workstream is archived. The store otherwise only ever grows: every session's
 * full transcript (messages, thinking, tool I/O) stays resident for the app's
 * lifetime. The backend has already killed the live drivers, and the SQLite
 * event log remains the source of truth if the workstream is restored, so
 * nothing is lost by releasing them here.
 */
export function removeSessionsForWorkstream(workstreamId: string): void {
  const { sessions } = useSessionStore.getState();
  const ids = Object.keys(sessions).filter((id) => sessions[id]?.workstreamId === workstreamId);
  if (ids.length === 0) return;
  for (const id of ids) channels.delete(id);
  useSessionStore.setState((s) => {
    const next = { ...s.sessions };
    for (const id of ids) delete next[id];
    return { sessions: next };
  });
}

/**
 * Epoch-ms start timestamp for each live background task (by `toolUseId`).
 * Stamped on first appearance, preserved across subsequent `setEntry` calls,
 * and naturally dropped when a task ends (it won't be in the live set anymore).
 * Mirrors the `workingSince` mechanism: maintained here (outside the pure
 * reducer) because it needs a clock.
 */
function nextTaskStartedAt(
  prev: SessionEntry | undefined,
  next: SessionEntry,
): Record<string, number> {
  const live = backgroundTasksFromItems(next.items);
  const out: Record<string, number> = {};
  for (const t of live) {
    out[t.toolUseId] = prev?.taskStartedAt?.[t.toolUseId] ?? Date.now();
  }
  return out;
}

/**
 * The working-start timestamp for the elapsed counter: stamp `now` the moment a
 * session begins working, keep it untouched while it stays working, clear it the
 * moment it stops. Recomputed on every `setEntry` so it's correct whichever path
 * changed the entry — live events, an optimistic send, or a permission answer.
 *
 * Keyed on [`isWorking`] (actively computing), not [`isBusy`]: a session parked
 * on a permission/question is busy but not working, so the counter pauses while
 * it waits on the user — and a restored, permission-held session never shows a
 * ticking timer (the restart bug).
 */
function nextWorkingSince(prev: SessionEntry | undefined, next: SessionEntry): number | null {
  if (!isWorking(next)) return null;
  const wasWorking = prev ? isWorking(prev) : false;
  return wasWorking ? (prev?.workingSince ?? Date.now()) : Date.now();
}

/**
 * A Tauri Channel that buffers events until we know which session they belong to.
 * Events can arrive before the start/resume command resolves (and we learn the
 * id), so they queue; `activate(id)` binds the channel and flushes the backlog in
 * order. Call `activate` only AFTER the session's entry exists, so `dispatch`
 * finds something to fold into. Shared by [`startSession`] and [`resumeSession`].
 */
function bufferedChannel(): {
  channel: Channel<SessionEvent>;
  activate: (sessionId: string) => void;
} {
  const buffer: SessionEvent[] = [];
  let boundId: string | null = null;
  const channel = new Channel<SessionEvent>();
  channel.onmessage = (event) => {
    if (boundId) dispatch(boundId, event);
    else buffer.push(event);
  };
  return {
    channel,
    activate(sessionId) {
      boundId = sessionId;
      for (const event of buffer) dispatch(sessionId, event);
      buffer.length = 0;
    },
  };
}

/** Fold one event into a session's transcript and run its side effects. */
function dispatch(sessionId: string, event: SessionEvent): void {
  const current = entryOf(sessionId);
  if (!current) return;

  // Permission-mode change: swallow the kill's `exited` so the session detaches
  // (live:false, idle) instead of flashing "session ended". Claude can't switch a
  // running session into bypass, so we relaunch lazily — the next message resumes
  // with the new mode (the proven resume-then-send path), rather than eagerly
  // resuming into an idle process that emits no `init` and hangs on "connecting".
  if (event.kind === "exited" && current.reloading) {
    channels.delete(sessionId);
    // The kill abandoned whatever turn was in flight, so cancel any permission
    // card or pending question it was parked on — its request id died with the
    // old process, so leaving it open would strand the session `isBusy` forever
    // (composer locked, queue frozen) on an ask no live claude can answer.
    setEntry(sessionId, {
      ...current,
      ...cancelPendingPrompts(current),
      live: false,
      reloading: false,
      state: "idle",
    });
    void queryClient.invalidateQueries({ queryKey: ["workstream", current.workstreamId, "diff"] });
    return;
  }

  const wasNeedsYou = classifyEntry(current) === "needs-you";
  const nextEntry: SessionEntry = { ...current, ...reduceEvent(current, event) };
  setEntry(sessionId, nextEntry);

  // Fire an OS toast when a session *transitions* into needs-you, so the user
  // notices an attention dot while the window is unfocused. Skip when the user
  // pre-allowed this tool (the auto-respond below clears it immediately) so we
  // don't toast for permissions the user never sees.
  const isNeedsYou = classifyEntry(nextEntry) === "needs-you";
  const autoApproved =
    event.kind === "permissionAsk" && current.autoAllow.includes(event.toolName);
  if (!wasNeedsYou && isNeedsYou && !autoApproved) {
    void maybeNotifyTransition(sessionId, current.workstreamId, current.stage);
  }

  if (event.kind === "permissionAsk" && current.autoAllow.includes(event.toolName)) {
    // "Allow for this agent" was chosen earlier — answer this one automatically.
    void respondPermission(sessionId, event.requestId, true, null);
  }
  if (event.kind === "turnDone" || event.kind === "exited") {
    // The agent likely touched the worktree; refresh the Changes panel
    // (git-status), any open diffs (file-sides), and the sidebar's line counts.
    invalidateWorktreeGit(current.workstreamId);
  }
  if (event.kind === "turnDone") {
    // The turn is free — deliver the next message queued while it ran.
    flushQueuedMessage(sessionId);
  }
  if (event.kind === "exited") {
    channels.delete(sessionId);
    const latest = entryOf(sessionId);
    if (latest) setEntry(sessionId, { ...latest, live: false });
    // Ahead/behind and link status may have changed (commits, pushes).
    void queryClient.invalidateQueries({ queryKey: ["workstreams"] });
  }
}

/** Fire the needs-you toast unless the user is already looking right at this
 *  session — i.e., the window is OS-focused AND the project route shows this
 *  workstream AND the session is bound to one of the active panes' tabs.
 *  Errs toward firing: any uncertainty (URL not canonical, tabs missing, the
 *  Tauri runtime missing in tests) is treated as not-visible. Always
 *  fire-and-forget — never lets a notification failure escape into dispatch. */
async function maybeNotifyTransition(
  sessionId: string,
  workstreamId: string,
  stage: string,
): Promise<void> {
  try {
    if (await isSessionVisible(sessionId, workstreamId)) return;
    const ws = findWorkstreamForNotify(workstreamId);
    if (!ws) return;
    await notifyNeedsAttention({
      workstreamId,
      projectPath: ws.projectPath,
      workstreamTitle: ws.title,
      stage,
    });
  } catch {
    /* Notifications are a side effect; a failure here must not perturb the
       dispatch path (jsdom has no Tauri runtime, e.g.). */
  }
}

async function isSessionVisible(sessionId: string, workstreamId: string): Promise<boolean> {
  if (!(await getCurrentWindow().isFocused())) return false;
  const loc = router.state.location;
  if (!loc.pathname.startsWith("/projects/")) return false;
  const search = loc.search as { item?: string };
  if (search.item !== workstreamId) return false;
  const tabs = useWorkstreamTabsStore.getState().byWs[workstreamId];
  if (!tabs) return false;
  const main = tabs.shells.find((s) => s.id === tabs.activeId);
  const right = tabs.rightSplit
    ? tabs.shells.find((s) => s.id === tabs.rightActiveId)
    : null;
  return main?.sessionId === sessionId || right?.sessionId === sessionId;
}

/**
 * Start a new agent session for a workstream stage and bind its event stream.
 * Returns the persisted [`Session`] row (its `id` becomes the tab's binding).
 */
export async function startSession(args: {
  workstreamId: string;
  stage: string;
  input: string;
  /** Files/library docs attached to the first turn. Their paths go to the
   *  backend prompt; they render as pin cards in the echo so no raw path shows. */
  attachments?: Attachment[];
  /** `--model` value, or null/undefined for claude's default. */
  model?: string | null;
  /** `--effort` level, or null/undefined for claude's default. */
  effort?: string | null;
  /** `--permission-mode` override, or null/undefined to use the stage default. */
  permissionMode?: string | null;
}): Promise<Session> {
  const { channel, activate } = bufferedChannel();
  const attachments = args.attachments ?? [];

  const session = await unwrap(
    commands.startStageSession(
      args.workstreamId,
      args.stage,
      args.input,
      attachments.map((a) => a.path),
      args.model ?? null,
      args.effort ?? null,
      args.permissionMode ?? null,
      channel,
    ),
  );

  // Synchronous from here: no `await`, so no buffered onmessage can interleave.
  channels.set(session.id, channel);

  let view = initialSessionView("starting");
  view = { ...view, model: session.model, claudeSessionId: session.claudeSessionId };
  // Echo the composed prompt (skill command + input) — but NOT the doc/path
  // list the backend appends; those render as pin cards instead, so the agent's
  // raw paths never appear in the user bubble.
  view = addUserMessage(
    view,
    composeStagePrompt(args.stage, args.input),
    args.stage,
    attachments,
    Date.now(),
  );
  setEntry(session.id, {
    ...view,
    workstreamId: args.workstreamId,
    stage: args.stage,
    autoAllow: [],
    live: true,
    permissionMode: args.permissionMode ?? null,
    workingSince: null,
    taskStartedAt: {},
    queued: [],
  });

  activate(session.id); // bind + flush any events that raced the command
  return session;
}

/**
 * Re-attach a live claude process to a restored (detached) session via
 * `--resume`, so the conversation continues after an app restart. Keeps the
 * existing transcript; new events append. No-op if already live.
 */
export async function resumeSession(sessionId: string): Promise<void> {
  const current = entryOf(sessionId);
  if (!current || channels.has(sessionId)) return;

  const { channel, activate } = bufferedChannel();
  channels.set(sessionId, channel);
  setEntry(sessionId, { ...current, live: true, state: "starting" });

  try {
    // Resume with claude's original session settings (model/effort null), but
    // re-apply the chosen permission mode — claude doesn't restore it, and it's
    // how a restart carries a mode (e.g. bypass) a live session can't switch to.
    await unwrap(commands.resumeSession(sessionId, null, null, current.permissionMode, channel));
  } catch (e) {
    channels.delete(sessionId);
    const latest = entryOf(sessionId);
    if (latest) setEntry(sessionId, { ...latest, live: false, state: "idle" });
    throw e;
  }

  activate(sessionId); // bind + flush events buffered during the resume call
}

/**
 * Restart a live session into a new permission mode via the backend's atomic
 * `restart_session` (kill the old process, wait for it to die, resume with the
 * new mode). Doing the kill+wait+respawn server-side removes the frontend race
 * where a resume could land on the still-dying old process and inherit its mode.
 *
 * Channel handling: the old process's kill-`exited` is still routed to the old
 * channel, so we drop that channel and flag `reloading` first — the dispatch
 * swallow turns that exit into a clean detach (no "session ended"). Only after
 * the backend has respawned do we bind the fresh channel, so the swallow can't
 * race-delete it. The resumed `claude` is alive but silent until the next
 * message, so we settle to idle/ready rather than hanging on "connecting".
 */
async function restartSession(sessionId: string, mode: string | null): Promise<void> {
  const current = entryOf(sessionId);
  if (!current) return;

  channels.delete(sessionId); // detach the old channel; its kill-exit is swallowed
  setEntry(sessionId, { ...current, permissionMode: mode, reloading: true, live: false, state: "starting" });

  const { channel, activate } = bufferedChannel();
  try {
    await unwrap(commands.restartSession(sessionId, mode, channel));
  } catch (e) {
    const latest = entryOf(sessionId);
    if (latest) setEntry(sessionId, { ...latest, reloading: false, live: false, state: "idle" });
    throw e;
  }

  channels.set(sessionId, channel); // bind the fresh stream now that the old one is gone
  activate(sessionId);
  const after = entryOf(sessionId);
  if (after) {
    // The restart abandoned the in-flight turn (the resumed claude is silent until
    // the next message), so cancel any permission/question it was parked on —
    // otherwise the session stays `isBusy` forever on a dead ask and the composer
    // (and the queue behind it) lock up with no way to recover. Settle to ready.
    setEntry(sessionId, {
      ...after,
      ...cancelPendingPrompts(after),
      live: true,
      reloading: false,
      streaming: false,
      state: "idle",
    });
    // The turn is free again — deliver the next queued follow-up under the new
    // mode, the same one-per-turn steering as a turn finishing normally.
    flushQueuedMessage(sessionId);
  }
}

/** The 1-based ordinal of the user-kind item at `itemId` among all user-kind
 *  items in the session. The backend's rewind/fork commands consume this
 *  ordinal (mapping it to the N-th `userMessage` event in the persisted log),
 *  so the frontend doesn't need to know about seqs. Returns 0 if not found —
 *  callers treat that as "no-op". */
function userTurnOrdinal(items: SessionEntry["items"], itemId: string): number {
  let n = 0;
  for (const item of items) {
    if (item.kind !== "user") continue;
    n += 1;
    if (item.id === itemId) return n;
  }
  return 0;
}

/**
 * Rewind a session to the user turn identified by `itemId`, dropping that turn
 * and everything after it — Claude-Desktop-style "rewind to here". Drives the
 * backend's `rewind_session`: the live driver is killed, persisted events at
 * the cutoff onward are deleted, and the session re-spawns with `--resume` so
 * the conversation can continue from the truncated state. The user can then
 * re-prompt from that point (the original prompt is gone from the transcript).
 */
export async function rewindToMessage(sessionId: string, itemId: string): Promise<void> {
  const current = entryOf(sessionId);
  if (!current) return;
  const ordinal = userTurnOrdinal(current.items, itemId);
  if (ordinal === 0) return;

  // Drop the old channel BEFORE the backend kills the live driver — the kill's
  // `exited` event is routed to the old channel, and we want it absorbed (the
  // session isn't ending; it's re-spawning), so flag it like `restartSession`.
  channels.delete(sessionId);
  setEntry(sessionId, { ...current, reloading: true, live: false, state: "starting" });

  const { channel, activate } = bufferedChannel();
  try {
    await unwrap(commands.rewindSession(sessionId, ordinal, channel));
  } catch (e) {
    const latest = entryOf(sessionId);
    if (latest) setEntry(sessionId, { ...latest, reloading: false, live: false, state: "idle" });
    throw e;
  }

  // Drop the truncated tail from the visible transcript immediately — the
  // backend has removed them from SQLite, and we don't want a stale post-cut
  // tail to linger until the next attach/restore cycle.
  //
  // Leave context-window / cost / turn-count fields untouched: claude's
  // actual session memory is unchanged by our rewind (the CLI has no rewind),
  // so the pre-rewind values ARE the current reality. The next assistant
  // message's `usage` event repopulates them naturally, so any drift here
  // self-corrects on the very next turn — no need to fake a reset.
  const latest = entryOf(sessionId);
  if (latest) {
    const truncated = truncateAtUserOrdinal(latest, ordinal);
    setEntry(sessionId, {
      ...latest,
      ...truncated,
      reloading: false,
      live: true,
      streaming: false,
      state: "running",
      pendingPermission: null,
      pendingQuestion: null,
      queued: [],
    });
  }

  channels.set(sessionId, channel);
  activate(sessionId);
}

/**
 * Drop every item from `view.items` at or after the N-th user turn, restoring
 * the view to the state right before that prompt was sent. The reducer
 * `counter` is left untouched (item ids are still unique).
 */
function truncateAtUserOrdinal(view: SessionView, ordinal: number): SessionView {
  let seen = 0;
  for (let i = 0; i < view.items.length; i += 1) {
    if (view.items[i].kind !== "user") continue;
    seen += 1;
    if (seen === ordinal) return { ...view, items: view.items.slice(0, i) };
  }
  return view;
}

/**
 * Change a session's permission mode. Claude can't switch a *running* session
 * into `bypassPermissions` (it's a launch-time decision), so applying a new mode
 * to an existing conversation means relaunching the process with the new flag,
 * continuing via `--resume` so the full transcript is preserved (see
 * [`restartSession`]). A not-live session just records the choice — it takes
 * effect on the next start/resume. The caller confirms first if a turn is in
 * flight (the restart interrupts it). `mode` is the `--permission-mode` flag, or
 * null for the stage default.
 */
export async function applyPermissionMode(
  sessionId: string,
  mode: string | null,
): Promise<void> {
  const current = entryOf(sessionId);
  if (!current || current.permissionMode === mode) return;

  if (!channels.has(sessionId)) {
    setEntry(sessionId, { ...current, permissionMode: mode });
    return;
  }
  await restartSession(sessionId, mode);
}

/**
 * Send a follow-up turn — or, if a live turn is already in flight, queue it for
 * delivery when that turn finishes, so the user can keep steering the
 * conversation without waiting (à la Claude Code / Conductor). A detached
 * (restored) session has nothing running to queue behind, so it's delivered
 * immediately, resuming the process first.
 */
export async function sendMessage(
  sessionId: string,
  text: string,
  attachments: Attachment[] = [],
): Promise<void> {
  const current = entryOf(sessionId);
  if (!current) return;
  if (channels.has(sessionId) && isBusy(current)) {
    setEntry(sessionId, { ...current, queued: [...current.queued, { text, attachments }] });
    return;
  }
  await deliverMessage(sessionId, text, attachments);
}

/** Actually send a turn: optimistic user bubble (text + attachment cards),
 *  resume if detached, hand to the driver. Attachment paths go to the backend
 *  as `attached_docs` (composed into the prompt server-side, so the agent can
 *  read them) but never into the bubble. The delivery half of [`sendMessage`];
 *  also used to flush the queue. */
async function deliverMessage(
  sessionId: string,
  text: string,
  attachments: Attachment[] = [],
): Promise<void> {
  const current = entryOf(sessionId);
  if (!current) return;
  const withUser = addUserMessage(current, text, null, attachments, Date.now());
  const optimisticId = withUser.items[withUser.items.length - 1].id;
  setEntry(sessionId, { ...current, ...withUser });
  if (!channels.has(sessionId)) await resumeSession(sessionId);
  try {
    await unwrap(commands.sendUserMessage(sessionId, text, attachments.map((a) => a.path)));
  } catch (e) {
    // The turn never reached the driver, so no `userMessage` event will be
    // persisted. Drop the optimistic bubble so the visible transcript stays in
    // lockstep with the log — an orphaned bubble would shift every later user
    // turn's ordinal and misaim a subsequent rewind.
    const latest = entryOf(sessionId);
    if (latest) {
      setEntry(sessionId, { ...latest, items: latest.items.filter((it) => it.id !== optimisticId) });
    }
    throw e;
  }
}

/**
 * Deliver the next queued message once a turn finishes (conversation steering).
 * One per turn so each queued message gets its own turn, in order; a no-op while
 * the session is still parked on a permission/question (the user owes an answer
 * first) — it retries on the next `turnDone`.
 */
function flushQueuedMessage(sessionId: string): void {
  const entry = entryOf(sessionId);
  if (!entry || entry.queued.length === 0 || isBusy(entry)) return;
  const [next, ...rest] = entry.queued;
  setEntry(sessionId, { ...entry, queued: rest });
  void deliverMessage(sessionId, next.text, next.attachments);
}

/** Drop a still-queued message before it's delivered (the user changed course). */
export function cancelQueuedMessage(sessionId: string, index: number): void {
  const current = entryOf(sessionId);
  if (!current) return;
  setEntry(sessionId, { ...current, queued: current.queued.filter((_, i) => i !== index) });
}

/** Answer a pending permission ask. */
export async function respondPermission(
  sessionId: string,
  requestId: string,
  allow: boolean,
  message: string | null,
): Promise<void> {
  const current = entryOf(sessionId);
  if (current) {
    setEntry(sessionId, {
      ...current,
      ...resolvePermission(current, requestId, allow ? "allow" : "deny"),
    });
  }
  // A restored (post-restart) session has no live process and its original
  // request id is dead, so the answer can't be delivered to it. Resume instead:
  // claude re-establishes the turn and re-issues the ask — auto-allowed if the
  // user chose "allow for this agent" (the tool is already in `autoAllow`).
  if (!channels.has(sessionId)) {
    await resumeSession(sessionId);
    return;
  }
  await unwrap(commands.respondPermission(sessionId, requestId, allow, message));
}

/**
 * Answer an `AskUserQuestion` prompt. `answers` maps each question's text to the
 * chosen answer string (multi-select choices joined by ", "); an empty map means
 * the user dismissed without answering. Optimistically un-parks the session,
 * then sends the answer to the driver.
 */
export async function answerQuestion(
  sessionId: string,
  requestId: string,
  answers: Record<string, string>,
): Promise<void> {
  const current = entryOf(sessionId);
  if (current) {
    setEntry(sessionId, { ...current, ...resolveQuestion(current, requestId) });
  }
  // Restored sessions are detached (see `respondPermission`): resume so claude
  // re-issues the question rather than sending an answer its dead process can't
  // receive.
  if (!channels.has(sessionId)) {
    await resumeSession(sessionId);
    return;
  }
  await unwrap(commands.answerQuestion(sessionId, requestId, JSON.stringify(answers)));
}

/** Allow this ask AND auto-approve future asks for the same tool this session. */
export async function allowForSession(
  sessionId: string,
  requestId: string,
  toolName: string,
): Promise<void> {
  const current = entryOf(sessionId);
  if (current && !current.autoAllow.includes(toolName)) {
    setEntry(sessionId, { ...current, autoAllow: [...current.autoAllow, toolName] });
  }
  await respondPermission(sessionId, requestId, true, null);
}

/** Gracefully interrupt the current turn (the session stays alive). */
export async function interruptSession(sessionId: string): Promise<void> {
  await unwrap(commands.interruptSession(sessionId));
}

/**
 * Stop the current turn for the composer's stop button. Sends claude's graceful
 * control-protocol interrupt first; if claude can't respond (model call wedged,
 * a tool hung mid-execution, etc.) and the session is still actively computing
 * after a short grace window, escalates to a hard kill so the user is never
 * stuck with a frozen turn and no way out. A detached session has no live turn
 * to stop. Sessions parked on a permission/question are already un-working, so
 * the watchdog resolves immediately and we don't kill — the interrupt still
 * delivers when they're answered.
 */
const STOP_ESCALATION_MS = 3000;
export async function stopSession(sessionId: string): Promise<void> {
  if (!channels.has(sessionId)) return;

  // A turn parked on a permission/question isn't computing — it's blocked on the
  // user, and claude is blocked on stdin waiting for that control response. "Stop"
  // here means cancel the ask: decline the permission (or dismiss the question) so
  // the live driver unblocks, and clear the card so the composer unlocks. The
  // decline goes out FIRST, while the request id is still live, so it lands even
  // if the interrupt below can't reach a claude that's still blocked on the ask.
  const parked = entryOf(sessionId);
  if (parked && isBusy(parked) && !isWorking(parked)) {
    const cleared = cancelPendingPrompts(parked);
    setEntry(sessionId, { ...parked, ...cleared, streaming: false, state: parkedState(cleared) });
    try {
      if (parked.pendingQuestion) {
        await unwrap(commands.answerQuestion(sessionId, parked.pendingQuestion.requestId, "{}"));
      } else {
        const requestId = openPermissionRequestId(parked);
        if (requestId) await unwrap(commands.respondPermission(sessionId, requestId, false, null));
      }
    } catch {
      // The driver may already be gone; the interrupt/kill below still runs.
    }
  }

  try {
    await interruptSession(sessionId);
  } catch {
    // Interrupt couldn't be delivered (stdin already closed, etc.). Fall through
    // to the watchdog: if the session is still working, the kill below ends it.
  }
  const settled = await waitForView(sessionId, (v) => !isWorking(v), STOP_ESCALATION_MS);
  if (settled) return;
  try {
    await killSession(sessionId);
  } catch {
    // Best-effort hard stop; the user can't do anything more from the composer.
  }
}

/** The request id of the permission ask a parked session owes an answer to: the
 *  tracked `pendingPermission`, else the first still-open card (a subagent can
 *  hold one open while `pendingPermission` tracks another). Null if none. */
function openPermissionRequestId(entry: SessionEntry): string | null {
  if (entry.pendingPermission) return entry.pendingPermission.requestId;
  for (const it of entry.items) {
    if (it.kind === "permission" && it.decision === null) return it.requestId;
  }
  return null;
}

/** Resolve as soon as the session's view matches `predicate`, or `false` if the
 *  timeout fires first. Used by [`stopSession`] to watch for the turn settling
 *  before escalating to a kill. */
function waitForView(
  sessionId: string,
  predicate: (view: SessionEntry) => boolean,
  timeoutMs: number,
): Promise<boolean> {
  const initial = entryOf(sessionId);
  if (!initial || predicate(initial)) return Promise.resolve(true);
  return new Promise((resolve) => {
    const finish = (result: boolean) => {
      clearTimeout(timer);
      unsub();
      resolve(result);
    };
    const timer = setTimeout(() => finish(false), timeoutMs);
    const unsub = useSessionStore.subscribe((state) => {
      const entry = state.sessions[sessionId];
      if (!entry || predicate(entry)) finish(true);
    });
  });
}

/** Hard-stop the session (kills the process tree). A detached (restored, not
 *  live) session has no process — just mark it ended locally. */
export async function killSession(sessionId: string): Promise<void> {
  if (!channels.has(sessionId)) {
    const current = entryOf(sessionId);
    if (current) {
      setEntry(sessionId, {
        ...current,
        ...reduceEvent(current, { kind: "exited", outcome: "killed", code: null, stderrTail: "" }),
        live: false,
      });
    }
    return;
  }
  await unwrap(commands.killSession(sessionId));
}

/**
 * Commit a workstream's changes from the header action: start a commit-stage
 * session — stage + commit only, no push or PR — in a dedicated, reusable
 * "Commit" tab and select it. Lets users checkpoint incrementally and open the
 * PR later via Ship. Reuses the live commit session's tab if one is running.
 */
export async function commitWorkstream(workstreamId: string): Promise<void> {
  const tabs = useWorkstreamTabsStore.getState().byWs[workstreamId];
  const commitTab = tabs?.shells.find((s) => s.mode === "commit");
  if (commitTab?.sessionId) {
    const entry = entryOf(commitTab.sessionId);
    if (entry && !isTerminal(entry.state)) {
      openCommitTab(workstreamId, commitTab.sessionId); // re-select the live commit tab
      return;
    }
  }
  const session = await startSession({
    workstreamId,
    stage: "commit",
    input: "",
  });
  openCommitTab(workstreamId, session.id);
}

/**
 * Ship a workstream from the header action (rather than a chat mode): start a
 * ship-stage session — commit, push, open a PR — in a dedicated, reusable
 * "Ship" tab and select it. If a ship session is already running for this
 * workstream, just focus its tab instead of spawning a second one.
 */
export async function shipWorkstream(workstreamId: string): Promise<void> {
  const tabs = useWorkstreamTabsStore.getState().byWs[workstreamId];
  const shipTab = tabs?.shells.find((s) => s.mode === "ship");
  if (shipTab?.sessionId) {
    const entry = entryOf(shipTab.sessionId);
    if (entry && !isTerminal(entry.state)) {
      openShipTab(workstreamId, shipTab.sessionId); // re-select the live ship tab
      return;
    }
  }
  const session = await startSession({
    workstreamId,
    stage: "ship",
    input: "",
  });
  openShipTab(workstreamId, session.id);
}

/**
 * Hand PR feedback back to the agent on the same branch. CI failures and
 * requested changes both route here from the PR status bar. Reuses the
 * workstream's "PR conversation" (the ship tab): if its session is still live,
 * the instruction is sent as a verbatim follow-up turn (preserving context);
 * otherwise a fresh `revise`-stage session is started (its first prompt is the
 * REVISE_PROMPT directive, never SHIP_PROMPT, so the agent won't open a 2nd PR).
 */
export async function addressPrFeedback(
  workstreamId: string,
  kind: "ci" | "comments",
  prNumber: number,
): Promise<void> {
  const instruction =
    kind === "ci"
      ? `CI checks are failing on PR #${prNumber}. Investigate the failures ` +
        "(`gh pr checks`, `gh run view --log-failed`), fix them, then commit and " +
        "push to this branch."
      : `Address the review feedback on PR #${prNumber}. Read it with ` +
        "`gh pr view --comments`, make the requested changes, then commit and push " +
        "to this branch.";

  const tabs = useWorkstreamTabsStore.getState().byWs[workstreamId];
  const shipTab = tabs?.shells.find((s) => s.mode === "ship");
  if (shipTab?.sessionId) {
    const entry = entryOf(shipTab.sessionId);
    if (entry && !isTerminal(entry.state)) {
      openShipTab(workstreamId, shipTab.sessionId); // focus the PR conversation
      await sendMessage(shipTab.sessionId, instruction);
      return;
    }
  }
  const session = await startSession({
    workstreamId,
    stage: "revise",
    input: instruction,
  });
  openShipTab(workstreamId, session.id);
}

/**
 * Rebuild a session's transcript from its persisted SQLite log when we have a
 * tab bound to it but no transcript in memory — i.e. after an app restart (the
 * tab binding survives in localStorage; the in-memory store does not).
 *
 * The claude child does not outlive the app (`kill_on_drop`), so a session we
 * don't already hold live is detached. If its log already ended terminally
 * (completed/crashed/killed) we keep it terminal; otherwise it was just left
 * mid-conversation — we mark it `idle` and `live: false`, so the next message
 * transparently resumes it (`--resume`) rather than forcing a new session. A
 * no-op if we already hold the transcript or a live channel.
 */
export async function restoreSessionIfNeeded(
  sessionId: string,
  workstreamId: string,
  stage: string,
  /** The tab's persisted `--permission-mode` flag (null = stage default), so a
   *  resume after an app restart re-applies the user's choice rather than
   *  silently dropping back to the stage default. */
  permissionMode: string | null = null,
): Promise<void> {
  if (entryOf(sessionId) || channels.has(sessionId) || restoring.has(sessionId)) return;
  restoring.add(sessionId);
  try {
    const log = await unwrap(commands.getSessionLog(sessionId));
    // A live start may have raced us while the log was loading.
    if (entryOf(sessionId) || channels.has(sessionId)) return;

    let view = initialSessionView("starting");
    for (const stored of log) {
      try {
        view = reduceEvent(view, JSON.parse(stored.payload) as SessionEvent);
        // The reducer is clock-free, so a replayed user turn lands with
        // `createdAt: null`. Backfill from the stored event's `createdAt`
        // (epoch ms) so the toolbar timestamp survives the restart.
        const last = view.items[view.items.length - 1];
        if (stored.kind === "userMessage" && last?.kind === "user" && last.createdAt === null) {
          view = { ...view, items: [...view.items.slice(0, -1), { ...last, createdAt: stored.createdAt }] };
        }
      } catch {
        // A single corrupt row shouldn't sink the whole transcript.
      }
    }
    // Settle the replayed transcript into a consistent resting state: a session
    // parked on a permission/question stays HELD (so the conversation pauses for
    // the user, rather than looking like a running turn); one cut off mid-turn
    // settles to idle. Answering a held prompt resumes the process — the request
    // id died with the old process, so the answer can't reach it directly, but
    // `respondPermission`/`answerQuestion` resume first and claude re-issues.
    view = restoredView(view);
    setEntry(sessionId, {
      ...view,
      workstreamId,
      stage,
      autoAllow: [],
      live: false,
      // The session log doesn't store the mode, but the tab does — re-apply it so
      // a resume keeps the user's choice (e.g. bypass) across an app restart.
      permissionMode,
      workingSince: null,
      taskStartedAt: {},
      queued: [],
    });
  } catch (error) {
    console.error(`failed to restore session ${sessionId}`, error);
  } finally {
    restoring.delete(sessionId);
  }
}

/** Plain-text transcript for "Copy transcript". */
export function transcriptToText(sessionId: string): string {
  const entry = useSessionStore.getState().sessions[sessionId];
  return entry ? viewToText(entry) : "(no transcript)";
}

/** Subscribe a component to one session's live state. */
export function useSession(sessionId: string | undefined): SessionEntry | undefined {
  return useSessionStore((s) => (sessionId ? s.sessions[sessionId] : undefined));
}

export type CreatedDoc = { path: string; kind: DocKind; title: string };

/**
 * Research/plan docs the agent created across all of a workstream's sessions,
 * newest last, deduped by path — derived from `Write`/`Edit` tool calls. Used to
 * surface docs for in-app review without leaving the project.
 */
export function useWorkstreamDocs(workstreamId: string | null): CreatedDoc[] {
  // Select a flat list of "kind\tpath" keys with shallow equality so streaming
  // events don't re-render this unless the set of docs actually changes.
  const keys = useSessionStore(
    useShallow((s) => {
      if (!workstreamId) return [] as string[];
      const seen = new Set<string>();
      const out: string[] = [];
      for (const entry of Object.values(s.sessions)) {
        if (entry.workstreamId !== workstreamId) continue;
        for (const item of entry.items) {
          if (item.kind !== "tool") continue;
          const doc = docFromTool(item.name, item.input);
          if (doc && !seen.has(doc.path)) {
            seen.add(doc.path);
            out.push(`${doc.kind}\t${doc.path}`);
          }
        }
      }
      return out;
    }),
  );
  return useMemo(
    () =>
      keys.map((k) => {
        const tab = k.indexOf("\t");
        const kind = k.slice(0, tab) as DocKind;
        const path = k.slice(tab + 1);
        return { kind, path, title: docTitle(path) };
      }),
    [keys],
  );
}

/**
 * The most recently created doc of `kind` in a workstream, or null.
 * Recency is defined by traversal order (session insertion order → transcript
 * order within a session) — the last matching write/edit wins. This is
 * deliberately NOT derived from `useWorkstreamDocs`: that selector dedups by
 * keeping the *first* occurrence of a path, whereas here a re-edited older doc
 * must win over a newer one of the same kind, so the two cannot be unified
 * without changing which doc gets suggested.
 */
export function useMostRecentDoc(
  workstreamId: string | null,
  kind: DocKind | null,
): CreatedDoc | null {
  const key = useSessionStore(
    useShallow((s) => {
      if (!workstreamId || !kind) return null;
      // Flatten every tool item across the workstream's sessions, project each to
      // the doc it created, keep those of the requested kind — the last in
      // traversal order is the most recent.
      const docs = Object.values(s.sessions)
        .filter((entry) => entry.workstreamId === workstreamId)
        .flatMap((entry) => entry.items)
        .flatMap((item) => (item.kind === "tool" ? [docFromTool(item.name, item.input)] : []))
        .filter((doc) => doc?.kind === kind);
      return docs[docs.length - 1]?.path ?? null;
    }),
  );
  return useMemo(
    () => (key ? { kind: kind as DocKind, path: key, title: docTitle(key) } : null),
    [key, kind],
  );
}

export type LiveBackgroundTask = BackgroundTask & { startedAt: number | null };

/**
 * Live background tasks for one session, with per-task elapsed timestamps.
 * Follows the `useWorkstreamDocs` pattern: a `useShallow` selector builds a
 * flat list of stable string keys so streaming deltas don't churn the bar
 * unless the task set or their statuses actually change; `useMemo` converts
 * the keys to typed objects only on real changes.
 *
 * `startedAt` is taken from `taskStartedAt` (maintained in the store layer
 * outside the pure reducer, like `workingSince`).
 */
export function useSessionBackgroundTasks(sessionId: string | undefined): LiveBackgroundTask[] {
  // Key: "toolUseId\0taskId\0status\0startedAt\0label\0command"
  // \0 is safe because none of these fields contain NUL bytes.
  const keys = useSessionStore(
    useShallow((s) => {
      if (!sessionId) return [] as string[];
      const entry = s.sessions[sessionId];
      if (!entry) return [] as string[];
      // A terminal session's process group is dead, so its background shells
      // are too — even though an un-stopped spawn still sits in `items`. Don't
      // surface phantom "running" tasks. (SessionChat also hides the bar behind
      // its `terminal` gate; this keeps the selector correct on its own.)
      if (isTerminal(entry.state)) return [] as string[];
      const tasks = backgroundTasksFromItems(entry.items);
      return tasks.map((t) => {
        const startedAt = entry.taskStartedAt[t.toolUseId] ?? 0;
        return [t.toolUseId, t.taskId ?? "", t.status, startedAt, t.label, t.command].join("\0");
      });
    }),
  );
  return useMemo(
    () =>
      keys.map((k) => {
        const [toolUseId, taskIdRaw, status, startedAtStr, label, ...cmdParts] = k.split("\0");
        return {
          toolUseId,
          taskId: taskIdRaw || null,
          status: status as BackgroundTaskStatus,
          startedAt: Number(startedAtStr) || null,
          label,
          command: cmdParts.join("\0"),
        } satisfies LiveBackgroundTask;
      }),
    [keys],
  );
}

/**
 * The single live-activity signal for a whole workstream, folded from the states
 * of every session bound to it. Drives the sidebar/list activity dot so you can
 * see at a glance which workstreams are running, which are blocked on you, and
 * which have finished a turn and want review — without opening each one.
 *
 * Categories, highest priority first (a workstream takes the most urgent state
 * any of its sessions is in):
 *  - `needs-you`  — a session is parked on a permission grant or a question; it
 *                   can't proceed until you answer. The thing you most want to
 *                   surface.
 *  - `working`    — a session is actively computing a turn.
 *  - `error`      — a session crashed or failed to spawn.
 *  - `review`     — a session finished its turn (ready) or completed; nothing is
 *                   running, but there's a result waiting for your eyes.
 *  - `idle`       — no session activity worth flagging (also the cold-start case
 *                   before a workstream's sessions are restored into the store).
 *  - `background` — a turn is done but one or more background shells are still
 *                   running; a distinct pulse distinct from the blue "working"
 *                   and the static green "review" dot.
 */
export type WorkstreamActivity = "needs-you" | "working" | "error" | "background" | "review" | "idle";

// Higher number = more urgent. A workstream surfaces the max across its sessions.
const ACTIVITY_RANK: Record<WorkstreamActivity, number> = {
  idle: 0,
  review: 1,
  background: 2,
  error: 3,
  working: 4,
  "needs-you": 5,
};

export function classifyEntry(entry: SessionEntry): WorkstreamActivity {
  if (
    entry.state === "awaiting_permission" ||
    entry.state === "awaiting_question" ||
    hasPendingPermission(entry) ||
    entry.pendingQuestion !== null
  ) {
    return "needs-you";
  }
  if (isWorking(entry)) return "working";
  if (entry.state === "crashed" || entry.state === "spawn_failed") return "error";
  // Turn finished but the session is alive and rests at "ready", or the session
  // ran to completion — either way there's a result to look at.
  if (entry.state === "completed" || (entry.state === "running" && !entry.streaming)) {
    // A live (non-terminal) session resting between turns with background
    // shells still running gets the distinct pulse instead of the static
    // "review" green. A "completed" session is terminal — its process group and
    // its shells are already dead — so it reads as "review", not "background".
    if (!isTerminal(entry.state) && backgroundTasksFromItems(entry.items).length > 0) {
      return "background";
    }
    return "review";
  }
  return "idle";
}

export function useWorkstreamActivity(workstreamId: string | null): WorkstreamActivity {
  return useSessionStore((s) => {
    if (!workstreamId) return "idle";
    let best: WorkstreamActivity = "idle";
    for (const entry of Object.values(s.sessions)) {
      if (entry.workstreamId !== workstreamId) continue;
      const cur = classifyEntry(entry);
      if (ACTIVITY_RANK[cur] > ACTIVITY_RANK[best]) best = cur;
    }
    return best;
  });
}

/**
 * Pure core of {@link useWorkstreamShipBusy} — true when one of this
 * workstream's Ship sessions is mid-turn ("working", or "needs-you" while paused
 * on a permission/question during that turn). Exported for testing.
 */
export function workstreamShipBusy(
  sessions: Record<string, SessionEntry>,
  workstreamId: string | null,
): boolean {
  if (!workstreamId) return false;
  for (const entry of Object.values(sessions)) {
    if (entry.workstreamId !== workstreamId || entry.stage !== "ship") continue;
    const activity = classifyEntry(entry);
    if (activity === "working" || activity === "needs-you") return true;
  }
  return false;
}

/**
 * True while this workstream's Ship session is mid-turn — actively
 * committing / pushing / opening the PR. Ship commits the worktree's current
 * changes itself, so the manual Commit button is redundant (and racy) while this
 * holds; Root hides it until the ship turn parks. By then the turn-done
 * invalidation has refreshed the live git-status count to 0, so Commit reappears
 * only for changes made *after* the ship, never for the ones Ship just
 * committed. Goes false once the ship turn finishes (parks at "review"/idle) or
 * the session ends, so a failed ship still leaves Commit available as a fallback.
 */
export function useWorkstreamShipBusy(workstreamId: string | null): boolean {
  return useSessionStore((s) => workstreamShipBusy(s.sessions, workstreamId));
}
