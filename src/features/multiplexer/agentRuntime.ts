//! Imperative lifecycle for Multiplexer PTY agents.
//!
//! Each agent gets a *persistent* detection Channel (held at module scope, like
//! the session store's live channels) so its state keeps updating, and a
//! blocked agent keeps badging + notifying, even while its tab is unmounted in
//! the background. The visible `AgentPtyTab` opens a SEPARATE render Channel for
//! xterm while it's on screen; the backend PTY broadcast feeds both subscribers.
//!
//! Spawning goes through `agent_open` (engine command in the worktree); reattach
//! reuses `terminal_attach`. Stop sends SIGINT; close kills the PTY. State
//! transitions are coalesced per frame before they hit the store.

import { Channel } from "@tauri-apps/api/core";

import { commands, type TerminalEvent } from "@/lib/bindings";
import { unwrap } from "@/lib/ipc";
import { notifyNeedsAttention } from "@/lib/notifications";
import { b64ToBytes } from "@/lib/terminal/terminalCodec";
import { StateCoalescer } from "./coalesce";
import { detectState, stripAnsi, type AgentState } from "./engineState";
import { engineChecksSessionOnDisk, type ResolvedEngine } from "./engines";
import {
  agentKey,
  removeAgent,
  setAgentStateBatch,
  setStoreBypass,
  upsertAgent,
  useMultiplexerStore,
} from "./multiplexerStore";

const QUIET_MS = 700;
const TAIL_CAP = 4000;
const DEFAULT_GRID = { cols: 120, rows: 32 };

type Runtime = {
  key: string;
  workstreamId: string;
  shellId: number;
  engine: ResolvedEngine;
  projectPath: string;
  workstreamTitle: string;
  terminalId?: string;
  detect?: Channel<TerminalEvent>;
  decoder: TextDecoder;
  tail: string;
  state: AgentState;
  quietTimer?: ReturnType<typeof setTimeout>;
  notifyOnBlocked: boolean;
  exited: boolean;
  starting?: Promise<string | null>;
  /** Source of truth for the bypass-permissions shield (the store mirrors it). */
  bypass: boolean;
};

const runtimes = new Map<string, Runtime>();

// One coalescer for all agents: flushes the per-frame batch into the store and
// fires a notification on each fresh transition into `blocked`.
const coalescer = new StateCoalescer((batch) => {
  const prev = useMultiplexerStore.getState().agents;
  for (const [key, state] of batch) {
    if (state === "blocked" && prev[key]?.state !== "blocked") {
      const rt = runtimes.get(key);
      if (rt) void maybeNotifyBlocked(rt);
    }
  }
  setAgentStateBatch(batch);
  // Re-assert bypass from the runtime (the source of truth) on every flush, so a
  // transient store change around a dock toggle / re-render can't leave the
  // shield out of sync, the next bit of output restores it.
  for (const key of batch.keys()) {
    const rt = runtimes.get(key);
    if (rt) setStoreBypass(rt.workstreamId, rt.shellId, rt.bypass);
  }
});

function recompute(rt: Runtime, quiet: boolean): void {
  const next = detectState(rt.engine.id, rt.tail, quiet, rt.state);
  if (next !== rt.state) {
    rt.state = next;
    coalescer.queue(rt.key, next);
  }
}

function onDetectEvent(rt: Runtime, ev: TerminalEvent): void {
  if (ev.kind === "output") {
    rt.tail = (rt.tail + stripAnsi(rt.decoder.decode(b64ToBytes(ev.data), { stream: true }))).slice(
      -TAIL_CAP,
    );
    recompute(rt, false); // live output → working (unless a prompt blocks)
    if (rt.quietTimer) clearTimeout(rt.quietTimer);
    rt.quietTimer = setTimeout(() => recompute(rt, true), QUIET_MS);
  } else if (ev.kind === "exited") {
    rt.exited = true;
    if (rt.quietTimer) clearTimeout(rt.quietTimer);
    if (rt.state !== "idle") {
      rt.state = "idle";
      coalescer.queue(rt.key, "idle");
    }
  }
}

async function maybeNotifyBlocked(rt: Runtime): Promise<void> {
  if (!rt.notifyOnBlocked) return;
  // Only toast when the window isn't focused, the tab + sidebar badge already
  // cover the in-view case, mirroring the native "needs you" behavior.
  try {
    if (document.hasFocus()) return;
  } catch {
    /* no document, shouldn't happen in the webview */
  }
  await notifyNeedsAttention({
    workstreamId: rt.workstreamId,
    projectPath: rt.projectPath,
    workstreamTitle: rt.workstreamTitle,
    stage: rt.engine.label,
  }).catch(() => {});
}

export type EnsureAgentOpts = {
  workstreamId: string;
  shellId: number;
  engine: ResolvedEngine;
  projectPath: string;
  workstreamTitle: string;
  /** True grid from the visible tab; falls back to a sane default for eager
   *  background spawns (the agent resizes when its tab is shown). */
  cols?: number;
  rows?: number;
  /** Persisted PTY id to reattach to before spawning fresh. */
  persistedTerminalId?: string;
  notifyOnBlocked: boolean;
  /** Whether this run is in bypass-permissions mode, restored from the persisted
   *  ShellTab on cold start so the shield (and a respawn's flags) survive a quit.
   *  Only seeds a freshly-created runtime; live runtimes keep their current flag. */
  bypass?: boolean;
  /** Whether a conversation has already begun, seeds a freshly-created runtime so
   *  a restored/resumed tab's Restart+shield resume immediately, while a fresh tab
   *  starts false and flips on the first keystroke. */
  started?: boolean;
};

/**
 * Ensure a PTY + detection stream exist for an agent. Idempotent and
 * concurrency-safe (in-flight spawns are de-duplicated). Returns the backend
 * terminal id, or null if the spawn failed.
 */
export function ensureAgent(opts: EnsureAgentOpts): Promise<string | null> {
  const key = agentKey(opts.workstreamId, opts.shellId);
  let rt = runtimes.get(key);

  if (rt) {
    rt.notifyOnBlocked = opts.notifyOnBlocked;
    rt.projectPath = opts.projectPath;
    rt.workstreamTitle = opts.workstreamTitle;
    if (rt.starting) return rt.starting;
    if (rt.terminalId && !rt.exited) return Promise.resolve(rt.terminalId);
  } else {
    rt = {
      key,
      workstreamId: opts.workstreamId,
      shellId: opts.shellId,
      engine: opts.engine,
      projectPath: opts.projectPath,
      workstreamTitle: opts.workstreamTitle,
      terminalId: opts.persistedTerminalId,
      decoder: new TextDecoder("utf-8"),
      tail: "",
      state: "idle",
      notifyOnBlocked: opts.notifyOnBlocked,
      exited: false,
      bypass: opts.bypass ?? false,
    };
    runtimes.set(key, rt);
    upsertAgent(opts.workstreamId, opts.shellId, {
      engineId: opts.engine.id,
      state: "idle",
      terminalId: opts.persistedTerminalId,
      bypass: opts.bypass ?? false,
      started: opts.started ?? false,
    });
  }

  const r = rt;
  const cols = opts.cols ?? DEFAULT_GRID.cols;
  const rows = opts.rows ?? DEFAULT_GRID.rows;

  const promise = (async (): Promise<string | null> => {
    // A Tauri Channel backs exactly one invoke: a failed terminal_attach tears
    // its channel down, so the agent_open fallback MUST get a fresh one or
    // detection is silently dead for every respawned agent (the
    // reference_tauri_channel_single_use trap).
    const makeDetect = () => {
      const ch = new Channel<TerminalEvent>();
      ch.onmessage = (ev) => onDetectEvent(r, ev);
      return ch;
    };
    // closeAgent may run mid-spawn (rapid create→close); if it did, the runtime
    // was removed from the map, adopt nothing and kill the PTY we just made so
    // it can't leak (orphaned process holding the worktree, unreachable from UI).
    const adoptOrKill = (id: string): string | null => {
      if (runtimes.get(key) !== r) {
        void commands.terminalClose(id);
        return null;
      }
      r.terminalId = id;
      r.exited = false;
      upsertAgent(r.workstreamId, r.shellId, { engineId: r.engine.id, terminalId: id });
      return id;
    };

    // Reattach to a still-live PTY first (survives webview reloads/remounts).
    const existing = r.terminalId ?? opts.persistedTerminalId;
    if (existing) {
      const ch = makeDetect();
      const ok = await unwrap(commands.terminalAttach(existing, ch)).catch(() => false);
      if (ok) {
        r.detect = ch;
        return adoptOrKill(existing);
      }
    }

    // Otherwise spawn the engine fresh in the worktree.
    try {
      const ch = makeDetect();
      const id = await unwrap(
        commands.agentOpen(r.workstreamId, r.engine.command, r.engine.args, cols, rows, ch),
      );
      r.detect = ch;
      return adoptOrKill(id);
    } catch {
      return null;
    }
  })();

  r.starting = promise;
  void promise.finally(() => {
    if (r.starting === promise) r.starting = undefined;
  });
  return promise;
}

/** The resume decision for a (re)launch: does THIS tab's conversation actually
 *  exist on disk? Authoritative for engines we can check (Claude/Codex/
 *  Antigravity), so a pinned-but-never-used id launches fresh instead of failing
 *  `--resume`. For others (Gemini/custom, or before an id is known) fall back to
 *  the caller's heuristic (whether the user has typed). */
export async function resolveResume(
  engineId: string,
  sessionId: string | undefined,
  workstreamId: string,
  fallback: boolean,
): Promise<boolean> {
  if (!sessionId || !engineChecksSessionOnDisk(engineId)) return fallback;
  return unwrap(commands.agentSessionExists(workstreamId, engineId, sessionId)).catch(() => fallback);
}

/** Push the notify-on-blocked preference to every live runtime, so toggling the
 *  setting affects already-running agents (not just newly spawned ones). */
export function setAllNotifyOnBlocked(value: boolean): void {
  for (const rt of runtimes.values()) rt.notifyOnBlocked = value;
}

/** Record whether an agent's current run is in bypass-permissions mode. Writes
 *  the runtime (the source of truth, re-asserted on every detection flush) and
 *  mirrors it into the store for the shield. */
export function setAgentBypass(workstreamId: string, shellId: number, bypass: boolean): void {
  const rt = runtimes.get(agentKey(workstreamId, shellId));
  if (rt) rt.bypass = bypass;
  setStoreBypass(workstreamId, shellId, bypass);
}

/** Kill the PTY and forget the agent, used when its tab is closed/archived. */
export function closeAgent(workstreamId: string, shellId: number): void {
  const key = agentKey(workstreamId, shellId);
  const rt = runtimes.get(key);
  if (rt) {
    if (rt.quietTimer) clearTimeout(rt.quietTimer);
    if (rt.terminalId) void commands.terminalClose(rt.terminalId);
    runtimes.delete(key);
  }
  removeAgent(workstreamId, shellId);
}
