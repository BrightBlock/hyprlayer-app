import { getCurrentWindow } from "@tauri-apps/api/window";
import { create } from "zustand";

import {
  commands,
  events,
  type ClientIdentity,
  type ClientRef,
  type SessionRevDto,
  type SessionReset,
  type SessionRevised,
} from "@/lib/bindings";
import type { EventBridge } from "@/lib/events";
import { unwrap } from "@/lib/ipc";
import { log } from "@/lib/log";
import {
  IDLE_AFTER_MS,
  isIdle,
  transition,
  type SyncEffect,
  type SyncInput,
  type SyncState,
  type TurnAuthor,
} from "./transcriptSync";

type TranscriptSyncStore = {
  bySession: Record<string, SyncState>;
  viewers: Record<string, ClientRef[]>;
  self: ClientIdentity | null;
  divider: Record<string, { seq: number; device: string } | null>;
};

export const useTranscriptSync = create<TranscriptSyncStore>(() => ({
  bySession: {},
  viewers: {},
  self: null,
  divider: {},
}));

/** specta types every f64 as `number | null`; a rev that crossed as a u64 is never null. */
export const rev = (v: number | null): number => v ?? 0;

// Registered by the session store, which imports this module.
export type SyncHandlers = {
  pull: (sessionId: string) => void;
  reload: (sessionId: string) => void;
  flush: (sessionId: string) => void;
  watched: () => string[];
  revised: (event: SessionRevised) => void;
  reset: (event: SessionReset) => void;
};

let handlers: SyncHandlers | null = null;

export function setSyncHandlers(next: SyncHandlers): void {
  handlers = next;
}

export function syncOf(sessionId: string): SyncState | undefined {
  return useTranscriptSync.getState().bySession[sessionId];
}

export function selfClientId(): string | null {
  return useTranscriptSync.getState().self?.clientId ?? null;
}

const SYNCED_FLASH_MS = 3_000;
const WATCH_DEBOUNCE_MS = 300;
const POLL_MS = 15_000;

const lastKeyAt = new Map<string, number>();
const drafting = new Map<string, boolean>();
const ownTurn = new Map<string, { streaming: boolean; turnAuthor: TurnAuthor }>();
const idleTimers = new Map<string, ReturnType<typeof setTimeout>>();
const retryTimers = new Map<string, ReturnType<typeof setTimeout>>();

const visible = () => document.visibilityState !== "hidden";

function idleNow(sessionId: string): boolean {
  const own = ownTurn.get(sessionId);
  return isIdle({
    now: Date.now(),
    lastKeyAt: lastKeyAt.get(sessionId) ?? 0,
    visible: visible(),
    streaming: own?.streaming ?? false,
    turnAuthor: own?.turnAuthor ?? null,
  });
}

function fresh(sessionId: string): SyncState {
  return {
    localRev: 0,
    committedRev: 0,
    remoteRev: 0,
    resetRev: 0,
    phase: "inSync",
    pendingTurns: [],
    authorDevice: null,
    workingDevice: null,
    flushable: false,
    draft: drafting.get(sessionId) ?? false,
    idle: idleNow(sessionId),
    fetching: false,
    retry: null,
    syncedWith: null,
    gone: false,
  };
}

// `settle` always allocates; an unchanged state must not re-render every subscriber.
function sameState(a: SyncState, b: SyncState): boolean {
  for (const key of Object.keys(a) as (keyof SyncState)[]) {
    if (a[key] === b[key]) continue;
    if (key === "pendingTurns" && a.pendingTurns.length === b.pendingTurns.length) {
      if (a.pendingTurns.every((t, i) => t === b.pendingTurns[i])) continue;
    }
    return false;
  }
  return true;
}

export function syncInput(sessionId: string, input: SyncInput): void {
  const existing = syncOf(sessionId);
  const before = existing ?? fresh(sessionId);
  const { state, effects } = transition(before, input);
  if (!existing || !sameState(before, state)) {
    useTranscriptSync.setState((s) => ({ bySession: { ...s.bySession, [sessionId]: state } }));
  }
  for (const effect of effects) runEffect(sessionId, effect);
  if (
    input.kind === "fetched" ||
    input.kind === "polled" ||
    (input.kind === "remote" && state.remoteRev <= state.localRev)
  ) {
    handlers?.flush(sessionId);
  }
}

function runEffect(sessionId: string, effect: SyncEffect): void {
  switch (effect.kind) {
    case "pull":
      handlers?.pull(sessionId);
      return;
    case "reload":
      handlers?.reload(sessionId);
      return;
    case "recheck":
      scheduleWatch();
      return;
    case "retryIn": {
      clearTimeout(retryTimers.get(sessionId));
      retryTimers.set(
        sessionId,
        setTimeout(() => {
          retryTimers.delete(sessionId);
          if (syncOf(sessionId)) syncInput(sessionId, { kind: "pull" });
        }, effect.ms),
      );
      return;
    }
    case "flashSynced": {
      const device = syncOf(sessionId)?.syncedWith ?? null;
      setTimeout(() => {
        const current = syncOf(sessionId);
        if (!current || current.syncedWith !== device) return;
        useTranscriptSync.setState((s) => ({
          bySession: { ...s.bySession, [sessionId]: { ...current, syncedWith: null } },
        }));
      }, SYNCED_FLASH_MS);
      return;
    }
  }
}

function evaluate(sessionId: string): void {
  const state = syncOf(sessionId);
  if (!state) return;
  const idle = idleNow(sessionId);
  const draft = drafting.get(sessionId) ?? false;
  if (state.idle !== idle || state.draft !== draft) syncInput(sessionId, { kind: "composer", draft, idle });
}

function evaluateAll(): void {
  for (const sessionId of Object.keys(useTranscriptSync.getState().bySession)) evaluate(sessionId);
}

/** `typed` is false when the draft changed without a keystroke (mount, a send clearing it, a recall). */
export function noteComposerInput(sessionId: string, draft: string, typed = true): void {
  drafting.set(sessionId, draft.trim() !== "");
  if (typed) {
    lastKeyAt.set(sessionId, Date.now());
    clearTimeout(idleTimers.get(sessionId));
    idleTimers.set(
      sessionId,
      setTimeout(() => {
        idleTimers.delete(sessionId);
        evaluate(sessionId);
      }, IDLE_AFTER_MS),
    );
  }
  evaluate(sessionId);
}

// Pushed on every store write, so it compares before it allocates.
export function noteOwnTurn(sessionId: string, streaming: boolean, turnAuthor: TurnAuthor): void {
  const prev = ownTurn.get(sessionId);
  if (prev && prev.streaming === streaming && prev.turnAuthor === turnAuthor) return;
  ownTurn.set(sessionId, { streaming, turnAuthor });
  evaluate(sessionId);
}

export function setDivider(sessionId: string, divider: { seq: number; device: string } | null): void {
  useTranscriptSync.setState((s) => ({ divider: { ...s.divider, [sessionId]: divider } }));
}

export function forgetSync(sessionId: string): void {
  for (const timers of [idleTimers, retryTimers]) {
    clearTimeout(timers.get(sessionId));
    timers.delete(sessionId);
  }
  lastKeyAt.delete(sessionId);
  drafting.delete(sessionId);
  ownTurn.delete(sessionId);
  useTranscriptSync.setState((s) => {
    if (!(sessionId in s.bySession) && !(sessionId in s.viewers) && !(sessionId in s.divider)) return s;
    const bySession = { ...s.bySession };
    const viewers = { ...s.viewers };
    const divider = { ...s.divider };
    delete bySession[sessionId];
    delete viewers[sessionId];
    delete divider[sessionId];
    return { bySession, viewers, divider };
  });
}

let supported: boolean | null = null;

/** Whether the connected server targets sync events at this client; errors read as no. */
export async function syncSupport(): Promise<boolean> {
  if (supported !== null) return supported;
  try {
    const status = await unwrap(commands.connectionStatus());
    // Only a live connection has announced; anything else is asked again.
    if (status.state === "connected") supported = status.transcriptSync;
    return status.transcriptSync;
  } catch {
    return false;
  }
}

export function forgetSyncSupport(): void {
  supported = null;
}

let installed = false;
let connected = true;
let watchTimer: ReturnType<typeof setTimeout> | null = null;

export function scheduleWatch(): void {
  if (!installed) return;
  if (watchTimer !== null) clearTimeout(watchTimer);
  watchTimer = setTimeout(() => {
    watchTimer = null;
    void sendWatch();
  }, WATCH_DEBOUNCE_MS);
}

async function sendWatch(): Promise<void> {
  if (!connected || !handlers || !(await syncSupport())) return;
  const ids = handlers.watched();
  const { bySession } = useTranscriptSync.getState();
  const sessions = ids.map((sessionId) => ({ sessionId, rev: bySession[sessionId]?.localRev ?? 0 }));
  let replies: SessionRevDto[];
  try {
    replies = await unwrap(commands.watchSessions(sessions));
  } catch (e) {
    log.warn("sync", "watching sessions failed", e);
    return;
  }
  const still = new Set(handlers.watched());
  const viewers: Record<string, ClientRef[]> = {};
  for (const reply of replies) viewers[reply.sessionId] = reply.viewers;
  useTranscriptSync.setState({ viewers });
  for (const reply of replies) {
    if (!still.has(reply.sessionId)) continue;
    syncInput(reply.sessionId, {
      kind: "polled",
      rev: rev(reply.rev),
      resetRev: rev(reply.resetRev),
      inFlight: reply.inFlight,
      working: reply.working,
    });
  }
  // A watched session the server no longer has was deleted.
  for (const sessionId of ids) {
    if (still.has(sessionId) && !(sessionId in viewers)) syncInput(sessionId, { kind: "gone" });
  }
}

/** Registered with `onServerReconnected`: the new connection announced afresh. */
export function transcriptSyncReconnected(): void {
  forgetSyncSupport();
  connected = true;
  scheduleWatch();
}

export function setupTranscriptSync(): void {
  installed = true;
  scheduleWatch();
  void unwrap(commands.getClientIdentity()).then(
    (self) => useTranscriptSync.setState({ self }),
    (e) => log.warn("sync", "loading this client's identity failed", e),
  );
  // WebKit doesn't always fire visibilitychange on occlusion; focus changes re-check too.
  document.addEventListener("visibilitychange", evaluateAll);
  void getCurrentWindow().onFocusChanged(evaluateAll);
  setInterval(() => {
    if (connected && visible()) void sendWatch();
  }, POLL_MS);
}

export const transcriptSyncBridge: EventBridge = () => [
  events.sessionRevised.listen(({ payload }) => handlers?.revised(payload)),
  events.sessionReset.listen(({ payload }) => handlers?.reset(payload)),
  events.sessionViewers.listen(({ payload }) =>
    useTranscriptSync.setState((s) => ({ viewers: { ...s.viewers, [payload.sessionId]: payload.viewers } })),
  ),
  events.serverDisconnected.listen(() => {
    connected = false;
    forgetSyncSupport();
  }),
];
