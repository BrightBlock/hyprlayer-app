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

import { Channel } from "@tauri-apps/api/core";
import { create } from "zustand";

import { commands, type Session, type SessionEvent } from "../../lib/bindings";
import { unwrap } from "../../lib/ipc";
import { queryClient } from "../../lib/queryClient";
import {
  addUserMessage,
  initialSessionView,
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
}): Promise<Session> {
  const buffer: SessionEvent[] = [];
  let boundId: string | null = null;
  const channel = new Channel<SessionEvent>();
  channel.onmessage = (event) => {
    if (boundId) dispatch(boundId, event);
    else buffer.push(event); // pre-id events; flushed once we know the session id
  };

  const session = await unwrap(
    commands.startStageSession(args.workstreamId, args.stage, args.input, args.attachedDocs, channel),
  );

  // Synchronous from here: no `await`, so no buffered onmessage can interleave.
  boundId = session.id;
  channels.set(session.id, channel);

  let view = initialSessionView("starting");
  view = { ...view, model: session.model, claudeSessionId: session.claudeSessionId };
  view = addUserMessage(view, args.input, args.stage);
  setEntry(session.id, {
    ...view,
    workstreamId: args.workstreamId,
    stage: args.stage,
    autoAllow: [],
  });

  for (const event of buffer) dispatch(session.id, event);
  buffer.length = 0;
  return session;
}

/** Send a follow-up turn to a live session. */
export async function sendMessage(sessionId: string, text: string): Promise<void> {
  const current = entryOf(sessionId);
  if (!current) return;
  setEntry(sessionId, { ...current, ...addUserMessage(current, text, null) });
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

/** Hard-stop the session (kills the process tree). */
export async function killSession(sessionId: string): Promise<void> {
  await unwrap(commands.killSession(sessionId));
}

/**
 * Rebuild a session's transcript from its persisted SQLite log when we have a
 * tab bound to it but no transcript in memory — i.e. after an app restart (the
 * tab binding survives in localStorage; the in-memory store does not).
 *
 * The claude child does not outlive the app (`kill_on_drop`), so a session we
 * don't already hold live is, by definition, historical: we replay the stored
 * events and force a terminal state if the log didn't already end on one (the
 * app was quit mid-session). No-op if we already hold the transcript or a live
 * channel, so this never duplicates or clobbers a running session.
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
    if (!isTerminal(view.state)) {
      // Process is gone (app was restarted) — close the session out.
      view = reduceEvent(view, { kind: "exited", outcome: "killed", code: null, stderrTail: "" });
    }
    setEntry(sessionId, { ...view, workstreamId, stage, autoAllow: [] });
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
