
import { create } from "zustand";

import type { PtyAgent, PtyAgentStatus } from "@/lib/domain/agentControl";
import { useWorkstreamTabsStore } from "@/features/session/workstreamTabs";
import type { AgentState } from "./engineState";
import { toAgentState } from "./ptyAgentState";

export type AgentRuntimeState = {
  workstreamId: string;
  shellId: number;
  engineId: string;
  state: AgentState;
  terminalId?: string;
  bypass?: boolean;
  started?: boolean;
  hooked?: boolean;
  sessionId?: string;
  activity?: string;
  /** What a hooked agent's CLI reported; the status bar shows it for the focused tab. */
  status?: PtyAgentStatus;
};

type Store = { agents: Record<string, AgentRuntimeState> };

export type PtyAgentApplied = { key: string; from: AgentState; to: AgentState };

// A hooked launch reports `starting` before its `pty_launch` response binds the terminal id.
const PARKED_CAP = 64;
const parked = new Map<string, PtyAgent>();
const launching = new Map<string, number>();

export function beginPtyLaunch(workstreamId: string): void {
  launching.set(workstreamId, (launching.get(workstreamId) ?? 0) + 1);
}

export function endPtyLaunch(workstreamId: string): void {
  const n = (launching.get(workstreamId) ?? 0) - 1;
  if (n > 0) launching.set(workstreamId, n);
  else launching.delete(workstreamId);
}

// Every desktop hears every PTY; parking only what this one will bind keeps other desktops' from evicting it.
function willBind(a: PtyAgent): boolean {
  if (a.spawnedBy || launching.has(a.workstreamId)) return true;
  const shells = useWorkstreamTabsStore.getState().byWs[a.workstreamId]?.shells ?? [];
  return shells.some((s) => s.ptyAgent?.terminalId === a.terminalId);
}

function park(a: PtyAgent): void {
  parked.delete(a.terminalId);
  parked.set(a.terminalId, a);
  if (parked.size > PARKED_CAP) parked.delete(parked.keys().next().value!);
}

export const agentKey = (workstreamId: string, shellId: number): string =>
  `${workstreamId}:${shellId}`;

export const useMultiplexerStore = create<Store>(() => ({ agents: {} }));

export function upsertAgent(
  workstreamId: string,
  shellId: number,
  patch: Partial<Omit<AgentRuntimeState, "workstreamId" | "shellId">>,
): void {
  const key = agentKey(workstreamId, shellId);
  useMultiplexerStore.setState((s) => {
    const cur = s.agents[key];
    const next: AgentRuntimeState = {
      workstreamId,
      shellId,
      engineId: patch.engineId ?? cur?.engineId ?? "",
      state: patch.state ?? cur?.state ?? "idle",
      terminalId: patch.terminalId ?? cur?.terminalId,
      bypass: patch.bypass ?? cur?.bypass,
      started: patch.started ?? cur?.started,
      hooked: patch.hooked ?? cur?.hooked,
      sessionId: patch.sessionId ?? cur?.sessionId,
      activity: patch.activity ?? cur?.activity,
      // A relaunch must not show the old process's model or context.
      status: patch.terminalId && patch.terminalId !== cur?.terminalId ? undefined : cur?.status,
    };
    if (
      cur &&
      cur.engineId === next.engineId &&
      cur.state === next.state &&
      cur.terminalId === next.terminalId &&
      cur.bypass === next.bypass &&
      cur.started === next.started &&
      cur.hooked === next.hooked &&
      cur.sessionId === next.sessionId &&
      cur.activity === next.activity &&
      cur.status === next.status
    ) {
      return s;
    }
    return { agents: { ...s.agents, [key]: next } };
  });
  const early = patch.terminalId ? parked.get(patch.terminalId) : undefined;
  if (early) {
    parked.delete(early.terminalId);
    applyPtyAgent(early);
  }
}

function sameStatus(a: PtyAgentStatus | undefined, b: PtyAgentStatus | undefined): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return (
    a.model === b.model &&
    a.effort === b.effort &&
    a.permissionMode === b.permissionMode &&
    a.planning === b.planning &&
    a.controllable === b.controllable &&
    a.context?.tokens === b.context?.tokens &&
    a.context?.window === b.context?.window &&
    a.context?.percent === b.context?.percent
  );
}

function keyForTerminal(
  agents: Record<string, AgentRuntimeState>,
  terminalId: string,
): string | undefined {
  for (const key in agents) if (agents[key].terminalId === terminalId) return key;
  return undefined;
}

/** Whether a tab on this desktop runs or launched that terminal, so this desktop's own rules notify for it. */
export function isTerminalBound(terminalId: string): boolean {
  return keyForTerminal(useMultiplexerStore.getState().agents, terminalId) !== undefined;
}

/** Applies a server-reported state; an unbound terminal is parked until `upsertAgent` binds it. */
export function applyPtyAgent(a: PtyAgent): PtyAgentApplied | undefined {
  const agents = useMultiplexerStore.getState().agents;
  const key = keyForTerminal(agents, a.terminalId);
  if (!key) {
    if (willBind(a)) park(a);
    return undefined;
  }
  const cur = agents[key];
  // `working` comes from UserPromptSubmit, so a turn ran even if no key was typed here.
  const started = !!cur.started || a.state === "working";
  const next: AgentRuntimeState = {
    ...cur,
    state: toAgentState(a.state, started),
    started,
    hooked: true,
    sessionId: a.providerSessionId ?? cur.sessionId,
    activity: a.activity ?? undefined,
    // Kept by reference when unchanged, so the tab's published controls stay put.
    status: sameStatus(cur.status, a.status) ? cur.status : a.status,
  };
  if (
    cur.state !== next.state ||
    !!cur.started !== started ||
    !cur.hooked ||
    cur.sessionId !== next.sessionId ||
    cur.activity !== next.activity ||
    cur.status !== next.status
  ) {
    useMultiplexerStore.setState((s) => ({ agents: { ...s.agents, [key]: next } }));
  }
  return { key, from: cur.state, to: next.state };
}

/** Replaces parked changes with the server's list; hooked agents it no longer knows went idle. */
export function applyPtyAgentSnapshot(list: PtyAgent[]): PtyAgentApplied[] {
  parked.clear();
  const live = new Set(list.map((a) => a.terminalId));
  const applied: PtyAgentApplied[] = [];
  for (const a of list) {
    const hit = applyPtyAgent(a);
    if (hit) applied.push(hit);
  }
  useMultiplexerStore.setState((s) => {
    let changed = false;
    const agents = { ...s.agents };
    for (const [key, cur] of Object.entries(agents)) {
      if (!cur.hooked || !cur.terminalId || live.has(cur.terminalId)) continue;
      if (cur.state === "idle" && cur.activity === undefined && cur.status === undefined) continue;
      agents[key] = { ...cur, state: "idle", activity: undefined, status: undefined };
      changed = true;
    }
    return changed ? { agents } : s;
  });
  return applied;
}

export function setStoreBypass(workstreamId: string, shellId: number, bypass: boolean): void {
  const key = agentKey(workstreamId, shellId);
  useMultiplexerStore.setState((s) => {
    const cur = s.agents[key];
    if (!cur || !!cur.bypass === bypass) return s;
    return { agents: { ...s.agents, [key]: { ...cur, bypass } } };
  });
}

export function markAgentStarted(workstreamId: string, shellId: number): void {
  const key = agentKey(workstreamId, shellId);
  useMultiplexerStore.setState((s) => {
    const cur = s.agents[key];
    if (!cur || cur.started) return s;
    return { agents: { ...s.agents, [key]: { ...cur, started: true } } };
  });
}

export function setAgentStateBatch(batch: Map<string, AgentState>): void {
  useMultiplexerStore.setState((s) => {
    let changed = false;
    const agents = { ...s.agents };
    for (const [key, state] of batch) {
      const cur = agents[key];
      if (cur && cur.state !== state) {
        agents[key] = { ...cur, state };
        changed = true;
      }
    }
    return changed ? { agents } : s;
  });
}

export function removeAgent(workstreamId: string, shellId: number): void {
  const key = agentKey(workstreamId, shellId);
  useMultiplexerStore.setState((s) => {
    if (!s.agents[key]) return s;
    const agents = { ...s.agents };
    delete agents[key];
    return { agents };
  });
}

export function useAgentState(
  workstreamId: string,
  shellId: number,
): AgentRuntimeState | undefined {
  return useMultiplexerStore((s) => s.agents[agentKey(workstreamId, shellId)]);
}

const PRIORITY: Record<AgentState, number> = { blocked: 3, working: 2, done: 1, idle: 0 };

export function rollupForWorkstream(
  agents: Record<string, AgentRuntimeState>,
  workstreamId: string,
): AgentState | undefined {
  let best: AgentState | undefined;
  for (const a of Object.values(agents)) {
    if (a.workstreamId !== workstreamId) continue;
    if (!best || PRIORITY[a.state] > PRIORITY[best]) best = a.state;
  }
  return best;
}

export function useWorkstreamAgentRollup(workstreamId: string): AgentState | undefined {
  return useMultiplexerStore((s) => rollupForWorkstream(s.agents, workstreamId));
}
