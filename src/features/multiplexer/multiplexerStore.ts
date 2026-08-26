
import { create } from "zustand";

import type { AgentState } from "./engineState";

export type AgentRuntimeState = {
  workstreamId: string;
  shellId: number;
  engineId: string;
  state: AgentState;
  terminalId?: string;
  bypass?: boolean;
  started?: boolean;
};

type Store = { agents: Record<string, AgentRuntimeState> };

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
    };
    if (
      cur &&
      cur.engineId === next.engineId &&
      cur.state === next.state &&
      cur.terminalId === next.terminalId &&
      cur.bypass === next.bypass &&
      cur.started === next.started
    ) {
      return s;
    }
    return { agents: { ...s.agents, [key]: next } };
  });
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
