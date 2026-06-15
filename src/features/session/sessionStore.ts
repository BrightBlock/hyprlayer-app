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
import { create } from "zustand";
import { useShallow } from "zustand/shallow";

import { commands, type Session, type SessionEvent } from "../../lib/bindings";
import { unwrap } from "../../lib/ipc";
import { queryClient } from "../../lib/queryClient";
import { docFromTool, docTitle, type DocKind } from "./toolDisplay";
import { composeStagePrompt } from "./stages";
import { openShipTab, useWorkstreamTabsStore } from "./workstreamTabs";
import {
  addUserMessage,
  initialSessionView,
  isBusy,
  isTerminal,
  reduceEvent,
  resolvePermission,
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
};

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
  useSessionStore.setState((s) => ({ sessions: { ...s.sessions, [sessionId]: next } }));
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
  setEntry(sessionId, { ...current, ...reduceEvent(current, event) });

  if (event.kind === "permissionAsk" && current.autoAllow.includes(event.toolName)) {
    // "Allow for this agent" was chosen earlier — answer this one automatically.
    void respondPermission(sessionId, event.requestId, true, null);
  }
  if (event.kind === "turnDone" || event.kind === "exited") {
    // The agent likely touched the worktree; refresh its diff.
    void queryClient.invalidateQueries({ queryKey: ["workstream", current.workstreamId, "diff"] });
  }
  if (event.kind === "exited") {
    channels.delete(sessionId);
    const latest = entryOf(sessionId);
    if (latest) setEntry(sessionId, { ...latest, live: false });
    // Ahead/behind and link status may have changed (commits, pushes).
    void queryClient.invalidateQueries({ queryKey: ["workstreams"] });
  }
}

/**
 * Start a new agent session for a workstream stage and bind its event stream.
 * Returns the persisted [`Session`] row (its `id` becomes the tab's binding).
 */
export async function startSession(args: {
  workstreamId: string;
  stage: string;
  input: string;
  attachedDocs: string[];
  /** `--model` value, or null/undefined for claude's default. */
  model?: string | null;
  /** `--effort` level, or null/undefined for claude's default. */
  effort?: string | null;
}): Promise<Session> {
  const { channel, activate } = bufferedChannel();

  const session = await unwrap(
    commands.startStageSession(
      args.workstreamId,
      args.stage,
      args.input,
      args.attachedDocs,
      args.model ?? null,
      args.effort ?? null,
      channel,
    ),
  );

  // Synchronous from here: no `await`, so no buffered onmessage can interleave.
  channels.set(session.id, channel);

  let view = initialSessionView("starting");
  view = { ...view, model: session.model, claudeSessionId: session.claudeSessionId };
  // Echo the composed prompt (skill command + input + docs), matching what the
  // backend actually sends to claude — not just the raw input.
  view = addUserMessage(
    view,
    composeStagePrompt(args.stage, args.input, args.attachedDocs),
    args.stage,
  );
  setEntry(session.id, {
    ...view,
    workstreamId: args.workstreamId,
    stage: args.stage,
    autoAllow: [],
    live: true,
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
    // Resume with claude's original session settings (model/effort null).
    await unwrap(commands.resumeSession(sessionId, null, null, channel));
  } catch (e) {
    channels.delete(sessionId);
    const latest = entryOf(sessionId);
    if (latest) setEntry(sessionId, { ...latest, live: false, state: "idle" });
    throw e;
  }

  activate(sessionId); // bind + flush events buffered during the resume call
}

/**
 * Send a follow-up turn. If the session was restored from the log and isn't
 * live (post-restart), transparently resume it first, then send.
 */
export async function sendMessage(sessionId: string, text: string): Promise<void> {
  const current = entryOf(sessionId);
  if (!current) return;
  setEntry(sessionId, { ...current, ...addUserMessage(current, text, null) });
  if (!channels.has(sessionId)) await resumeSession(sessionId);
  await unwrap(commands.sendUserMessage(sessionId, text));
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
  await unwrap(commands.respondPermission(sessionId, requestId, allow, message));
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

/** Grace period for a graceful interrupt to land before "stop" escalates to a
 *  hard kill. A turn deep in a subagent may never ack the control-protocol
 *  interrupt, so the stop button must always be able to actually stop. */
const INTERRUPT_GRACE_MS = 4000;

/**
 * Stop the current turn for the composer's stop button. Tries a graceful
 * interrupt first (keeps the process so the user can send another turn); if the
 * turn is still running after a grace period — e.g. claude is wedged in a
 * subagent and never acked — hard-kills the process tree. Sessions are
 * resumable, so the kill is recoverable.
 */
export async function stopSession(sessionId: string): Promise<void> {
  try {
    await interruptSession(sessionId);
  } catch {
    // Interrupt couldn't be delivered (stdin already closed, etc.) — fall
    // through to the kill below rather than leaving the turn wedged.
  }
  await new Promise((resolve) => setTimeout(resolve, INTERRUPT_GRACE_MS));
  const current = entryOf(sessionId);
  if (current && channels.has(sessionId) && isBusy(current)) {
    await killSession(sessionId);
  }
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
    attachedDocs: [],
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
      } catch {
        // A single corrupt row shouldn't sink the whole transcript.
      }
    }
    // Left mid-conversation (not a real terminal exit) → resumable, not ended.
    if (!isTerminal(view.state)) view = { ...view, state: "idle", streaming: false };
    setEntry(sessionId, { ...view, workstreamId, stage, autoAllow: [], live: false });
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
