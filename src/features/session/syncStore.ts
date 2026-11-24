import { focusManager } from "@tanstack/react-query";
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
import { transition, type HeldView, type SyncEffect, type SyncInput, type SyncState } from "./transcriptSync";

type TranscriptSyncStore = {
  bySession: Record<string, SyncState>;
  viewers: Record<string, ClientRef[]>;
  self: ClientIdentity | null;
  divider: Record<string, { seq: number; device: string } | null>;
  held: Record<string, HeldView>;
};

export const useTranscriptSync = create<TranscriptSyncStore>(() => ({
  bySession: {},
  viewers: {},
  self: null,
  divider: {},
  held: {},
}));

/** specta types every f64 as `number | null`; a rev that crossed as a u64 is never null. */
export const rev = (v: number | null): number => v ?? 0;

// Registered by the session store, which imports this module.
type SyncHandlers = {
  pull: (sessionId: string) => void;
  reload: (sessionId: string) => void;
  flush: (sessionId: string) => void;
  watched: () => string[];
  polled: (sessionId: string) => void;
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
const IDLE_POLL_MS = 60_000;

const drafting = new Map<string, boolean>();
const retryTimers = new Map<string, ReturnType<typeof setTimeout>>();

const visible = () => document.visibilityState !== "hidden";

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
  if (state.remoteRev > before.remoteRev && state.remoteRev > state.localRev) {
    const dismissedRev = useTranscriptSync.getState().held[sessionId]?.dismissedRev ?? null;
    setHeld(sessionId, { at: Date.now(), dismissedRev });
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

export function noteComposerInput(sessionId: string, draft: string): void {
  const next = draft.trim() !== "";
  drafting.set(sessionId, next);
  const state = syncOf(sessionId);
  if (state && state.draft !== next) syncInput(sessionId, { kind: "composer", draft: next });
}

export function setDivider(sessionId: string, divider: { seq: number; device: string } | null): void {
  useTranscriptSync.setState((s) => ({ divider: { ...s.divider, [sessionId]: divider } }));
}

function setHeld(sessionId: string, view: HeldView): void {
  useTranscriptSync.setState((s) => ({ held: { ...s.held, [sessionId]: view } }));
}

/** Hides the new-turns strip until a later revision lands. */
export function dismissHeld(sessionId: string): void {
  const remoteRev = syncOf(sessionId)?.remoteRev ?? 0;
  setHeld(sessionId, { at: useTranscriptSync.getState().held[sessionId]?.at ?? null, dismissedRev: remoteRev });
}

export function forgetSync(sessionId: string): void {
  clearTimeout(retryTimers.get(sessionId));
  retryTimers.delete(sessionId);
  drafting.delete(sessionId);
  useTranscriptSync.setState((s) => {
    if (![s.bySession, s.viewers, s.divider, s.held].some((m) => sessionId in m)) return s;
    const bySession = { ...s.bySession };
    const viewers = { ...s.viewers };
    const divider = { ...s.divider };
    const held = { ...s.held };
    delete bySession[sessionId];
    delete viewers[sessionId];
    delete divider[sessionId];
    delete held[sessionId];
    return { bySession, viewers, divider, held };
  });
}

let supported: boolean | null = null;
let tailRefused = false;

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
  tailRefused = false;
}

/** False once this connection's server refused a tail read, as one from before them does. */
export function tailReads(): boolean {
  return !tailRefused;
}

export function refuseTailReads(): void {
  tailRefused = true;
}

let installed = false;
let connected = true;
let watchTimer: ReturnType<typeof setTimeout> | null = null;
let pollTimer: ReturnType<typeof setTimeout> | null = null;
let pollMs = POLL_MS;
// The server replaces a connection's watched set, so an empty one goes out once to clear it.
let serverWatching = false;

export function scheduleWatch(): void {
  if (!installed) return;
  pollMs = POLL_MS;
  if (watchTimer !== null) clearTimeout(watchTimer);
  watchTimer = setTimeout(() => {
    watchTimer = null;
    void sendWatch();
  }, WATCH_DEBOUNCE_MS);
}

function schedulePoll(): void {
  if (pollTimer !== null) clearTimeout(pollTimer);
  pollTimer = setTimeout(() => {
    pollTimer = null;
    if (connected && visible()) void sendWatch();
    else schedulePoll();
  }, pollMs);
}

async function sendWatch(): Promise<void> {
  // Armed first, so a reply that never comes can't end the chain.
  schedulePoll();
  try {
    await watchOnce();
  } finally {
    schedulePoll();
  }
}

// A reply naming a revision or a turn this client hadn't heard of.
function stirred(reply: SessionRevDto): boolean {
  const known = syncOf(reply.sessionId);
  const heard = Math.max(known?.localRev ?? 0, known?.remoteRev ?? 0);
  return reply.working || reply.inFlight > 0 || rev(reply.rev) > heard || rev(reply.resetRev) > (known?.resetRev ?? 0);
}

async function watchOnce(): Promise<void> {
  if (!connected || !handlers) return;
  const ids = handlers.watched();
  if (ids.length === 0 && !serverWatching) return;
  if (!(await syncSupport())) return;
  const { bySession } = useTranscriptSync.getState();
  const sessions = ids.map((sessionId) => ({ sessionId, rev: bySession[sessionId]?.localRev ?? 0 }));
  let replies: SessionRevDto[];
  try {
    replies = await unwrap(commands.watchSessions(sessions));
  } catch (e) {
    log.warn("sync", "watching sessions failed", e);
    return;
  }
  serverWatching = ids.length > 0;
  pollMs = replies.some(stirred) ? POLL_MS : Math.min(pollMs * 2, IDLE_POLL_MS);
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
    // A running turn is read once it ends; an exit leaves the session not working.
    if (!reply.working) handlers.polled(reply.sessionId);
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
  // A new connection starts with nothing watched.
  serverWatching = false;
  scheduleWatch();
}

export function setupTranscriptSync(): void {
  installed = true;
  scheduleWatch();
  void unwrap(commands.getClientIdentity()).then(
    (self) => useTranscriptSync.setState({ self }),
    (e) => log.warn("sync", "loading this client's identity failed", e),
  );
  // An idle poll can be a minute out; a returning user shouldn't wait for it.
  focusManager.subscribe((focused) => {
    if (focused) scheduleWatch();
  });
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
