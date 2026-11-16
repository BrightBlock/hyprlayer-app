
import { Channel } from "@tauri-apps/api/core";

import { commands, type TerminalEvent } from "@/lib/bindings";
import type { PtyLaunched } from "@/lib/domain/agentControl";
import { unwrap } from "@/lib/ipc";
import { log } from "@/lib/log";
import { notifyNeedsAttention } from "@/lib/notifications";
import { releaseChannels } from "@/lib/terminal/channelRelease";
import { b64ToBytes } from "@/lib/terminal/terminalCodec";
import { bindPtyTabTerminal } from "@/features/session/workstreamTabs";
import { StateCoalescer } from "./coalesce";
import { detectState, stripAnsi, type AgentState } from "./engineState";
import { engineChecksSessionOnDisk, type ResolvedEngine } from "./engines";
import {
  agentKey,
  beginPtyLaunch,
  endPtyLaunch,
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
  starting?: Promise<AgentLaunch | null>;
  bypass: boolean;
  hooked: boolean;
};

/** `exited`: an adopted terminal that is gone; the tab shows it ended instead of launching a copy. */
export type AgentLaunch = { terminalId: string; providerSessionId?: string; exited?: boolean };

const runtimes = new Map<string, Runtime>();

const coalescer = new StateCoalescer((batch) => {
  const prev = useMultiplexerStore.getState().agents;
  for (const [key, state] of batch) {
    // A regex verdict queued before the hooks were known must not overwrite them.
    if (prev[key]?.hooked) {
      batch.delete(key);
      continue;
    }
    if (state === "blocked" && prev[key]?.state !== "blocked") void maybeNotifyBlocked(key);
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

// A reattached runtime learns it is hooked from the store, after the server reports it.
function isHooked(rt: Runtime): boolean {
  if (!rt.hooked && useMultiplexerStore.getState().agents[rt.key]?.hooked) {
    rt.hooked = true;
    rt.tail = "";
    if (rt.quietTimer) clearTimeout(rt.quietTimer);
  }
  return rt.hooked;
}

function onDetectEvent(rt: Runtime, ev: TerminalEvent): void {
  if (ev.kind === "output") {
    if (isHooked(rt)) return;
    rt.tail = (rt.tail + stripAnsi(rt.decoder.decode(b64ToBytes(ev.data), { stream: true }))).slice(
      -TAIL_CAP,
    );
    recompute(rt, false);
    if (rt.quietTimer) clearTimeout(rt.quietTimer);
    rt.quietTimer = setTimeout(() => recompute(rt, true), QUIET_MS);
  } else if (ev.kind === "exited") {
    rt.exited = true;
    if (rt.quietTimer) clearTimeout(rt.quietTimer);
    if (isHooked(rt)) {
      // A closed or restarted tab's old terminal must not recreate or overwrite its entry.
      if (runtimes.get(rt.key) === rt) upsertAgent(rt.workstreamId, rt.shellId, { state: "idle" });
    } else if (rt.state !== "idle") {
      rt.state = "idle";
      coalescer.queue(rt.key, "idle");
    }
  }
}

const goneListeners = new Map<string, Set<(terminalId: string) => void>>();

/** A terminal the server lost without an `exited` event (its server died); the tab's own channel stays silent. */
export function onAgentTerminalGone(
  workstreamId: string,
  shellId: number,
  listener: (terminalId: string) => void,
): () => void {
  const key = agentKey(workstreamId, shellId);
  const set = goneListeners.get(key) ?? new Set();
  set.add(listener);
  goneListeners.set(key, set);
  return () => {
    set.delete(listener);
    if (set.size === 0 && goneListeners.get(key) === set) goneListeners.delete(key);
  };
}

/**
 * After a reconnect: a failed attach is the server's own terminal registry saying the PTY is gone,
 * for hooked and unhooked agents alike. A live one keeps the new stream, since the old one died with the connection.
 */
export async function reconcileAgentTerminals(): Promise<void> {
  await Promise.all(
    [...runtimes.values()].map(async (rt) => {
      const id = rt.terminalId;
      if (!id || rt.exited || rt.starting) return;
      const ch = new Channel<TerminalEvent>();
      ch.onmessage = (ev) => onDetectEvent(rt, ev);
      const alive = await unwrap(commands.terminalAttach(id, ch)).catch(() => undefined);
      const current = runtimes.get(rt.key) === rt && rt.terminalId === id && !rt.exited;
      if (alive && current) {
        if (rt.detect) releaseChannels([rt.detect]);
        rt.detect = ch;
        return;
      }
      releaseChannels([ch]);
      if (alive === false && current) {
        onDetectEvent(rt, { kind: "exited", code: null });
        goneListeners.get(rt.key)?.forEach((fn) => fn(id));
      }
    }),
  );
}

export async function maybeNotifyBlocked(key: string): Promise<void> {
  const rt = runtimes.get(key);
  if (!rt?.notifyOnBlocked) return;
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

export type AgentLaunchSession = {
  sessionId?: string;
  resume: boolean;
  bypass: boolean;
  model?: string;
  permissionMode?: string;
  effort?: string;
};

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
  session: AgentLaunchSession;
  started?: boolean;
  adopted?: boolean;
};

export function ensureAgent(opts: EnsureAgentOpts): Promise<AgentLaunch | null> {
  const key = agentKey(opts.workstreamId, opts.shellId);
  let rt = runtimes.get(key);

  if (rt) {
    rt.notifyOnBlocked = opts.notifyOnBlocked;
    rt.projectPath = opts.projectPath;
    rt.workstreamTitle = opts.workstreamTitle;
    if (rt.starting) return rt.starting;
    if (rt.terminalId && !rt.exited) return Promise.resolve({ terminalId: rt.terminalId });
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
      bypass: opts.session.bypass,
      hooked: false,
    };
    runtimes.set(key, rt);
    upsertAgent(opts.workstreamId, opts.shellId, {
      engineId: opts.engine.id,
      state: "idle",
      terminalId: opts.persistedTerminalId,
      bypass: opts.session.bypass,
      started: opts.started ?? false,
    });
  }

  const r = rt;
  const cols = opts.cols ?? DEFAULT_GRID.cols;
  const rows = opts.rows ?? DEFAULT_GRID.rows;

  const promise = (async (): Promise<AgentLaunch | null> => {
    const makeDetect = () => {
      const ch = new Channel<TerminalEvent>();
      ch.onmessage = (ev) => onDetectEvent(r, ev);
      return ch;
    };
    const adoptOrKill = (id: string, launched?: PtyLaunched): AgentLaunch | null => {
      if (runtimes.get(key) !== r) {
        // Only a launch of ours: a reattached terminal may be an adopted one other desktops share.
        if (launched) void commands.terminalClose(id);
        return null;
      }
      r.terminalId = id;
      r.exited = false;
      if (launched) r.hooked = launched.hooked;
      const providerSessionId = launched?.providerSessionId ?? undefined;
      upsertAgent(r.workstreamId, r.shellId, {
        engineId: r.engine.id,
        terminalId: id,
        hooked: launched?.hooked,
        sessionId: providerSessionId,
      });
      // A background tab is never mounted to persist the id, so the next reload would launch again.
      const prev = bindPtyTabTerminal(r.workstreamId, r.shellId, id, providerSessionId);
      if (launched && prev?.terminalId && prev.terminalId !== id && !prev.adopted) {
        void commands.terminalClose(prev.terminalId);
      }
      return { terminalId: id, providerSessionId };
    };

    const existing = r.terminalId ?? opts.persistedTerminalId;
    if (existing) {
      const ch = makeDetect();
      const ok = await unwrap(commands.terminalAttach(existing, ch)).catch((e) => {
        log.warn("agent", `reattaching terminal ${existing} failed`, e);
        return undefined;
      });
      if (ok) {
        r.detect = ch;
        return adoptOrKill(existing);
      }
      releaseChannels([ch]);
      // Resuming a shared agent here would start a second copy; only Restart launches one.
      if (opts.adopted) {
        // A failed call isn't the server saying it's gone; reconcile rechecks it after a reconnect.
        if (ok === undefined || runtimes.get(key) !== r) return null;
        r.terminalId = existing;
        onDetectEvent(r, { kind: "exited", code: null });
        return { terminalId: existing, exited: true };
      }
    }

    beginPtyLaunch(r.workstreamId);
    try {
      const ch = makeDetect();
      const { sessionId, resume, bypass, model, permissionMode, effort } = opts.session;
      const launched = await unwrap(
        commands.ptyLaunch(
          r.workstreamId,
          r.engine.id,
          {
            sessionId: sessionId ?? null,
            resume,
            bypass,
            model: model ?? null,
            permissionMode: permissionMode ?? null,
            effort: effort ?? null,
          },
          cols,
          rows,
          ch,
        ),
      );
      r.detect = ch;
      return adoptOrKill(launched.terminalId, launched);
    } catch (e) {
      log.error("agent", `starting ${r.engine.id} failed`, e);
      return null;
    } finally {
      endPtyLaunch(r.workstreamId);
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

function dropRuntime(workstreamId: string, shellId: number): Runtime | undefined {
  const key = agentKey(workstreamId, shellId);
  const rt = runtimes.get(key);
  if (rt) {
    if (rt.quietTimer) clearTimeout(rt.quietTimer);
    if (rt.detect) releaseChannels([rt.detect]);
    runtimes.delete(key);
  }
  removeAgent(workstreamId, shellId);
  return rt;
}

/** An `adopted` PTY belongs to every desktop: closing its tab detaches and leaves the agent running. */
export function closeAgent(workstreamId: string, shellId: number, adopted = false): void {
  const rt = dropRuntime(workstreamId, shellId);
  if (rt?.terminalId && !adopted) void commands.terminalClose(rt.terminalId);
}

/** Archive ends the agent, adopted or not; a tab never shown here has no runtime, only its stored terminal. */
export function endAgent(workstreamId: string, shellId: number, storedTerminalId?: string): void {
  const terminalId = dropRuntime(workstreamId, shellId)?.terminalId ?? storedTerminalId;
  if (terminalId) void commands.terminalClose(terminalId);
}
