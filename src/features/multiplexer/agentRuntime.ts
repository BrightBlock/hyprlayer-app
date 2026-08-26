
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
  bypass: boolean;
};

const runtimes = new Map<string, Runtime>();

const coalescer = new StateCoalescer((batch) => {
  const prev = useMultiplexerStore.getState().agents;
  for (const [key, state] of batch) {
    if (state === "blocked" && prev[key]?.state !== "blocked") {
      const rt = runtimes.get(key);
      if (rt) void maybeNotifyBlocked(rt);
    }
  }
  setAgentStateBatch(batch);
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
    recompute(rt, false);
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
  try {
    if (document.hasFocus()) return;
  } catch {
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
  cols?: number;
  rows?: number;
  persistedTerminalId?: string;
  notifyOnBlocked: boolean;
  bypass?: boolean;
  started?: boolean;
};

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
    const makeDetect = () => {
      const ch = new Channel<TerminalEvent>();
      ch.onmessage = (ev) => onDetectEvent(r, ev);
      return ch;
    };
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

    const existing = r.terminalId ?? opts.persistedTerminalId;
    if (existing) {
      const ch = makeDetect();
      const ok = await unwrap(commands.terminalAttach(existing, ch)).catch(() => false);
      if (ok) {
        r.detect = ch;
        return adoptOrKill(existing);
      }
    }

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

export async function resolveResume(
  engineId: string,
  sessionId: string | undefined,
  workstreamId: string,
  fallback: boolean,
): Promise<boolean> {
  if (!sessionId || !engineChecksSessionOnDisk(engineId)) return fallback;
  return unwrap(commands.agentSessionExists(workstreamId, engineId, sessionId)).catch(() => fallback);
}

export function setAllNotifyOnBlocked(value: boolean): void {
  for (const rt of runtimes.values()) rt.notifyOnBlocked = value;
}

export function setAgentBypass(workstreamId: string, shellId: number, bypass: boolean): void {
  const rt = runtimes.get(agentKey(workstreamId, shellId));
  if (rt) rt.bypass = bypass;
  setStoreBypass(workstreamId, shellId, bypass);
}

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
