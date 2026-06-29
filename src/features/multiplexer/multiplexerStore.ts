//! Reactive per-agent runtime state for Multiplexer mode.
//!
//! Holds the live `working/blocked/done/idle` state for every PTY agent, keyed
//! by `workstreamId:shellId`. Tab pills, the agent status bar, and the sidebar
//! rollup subscribe here. The imperative side (PTY channels, detection timers)
//! lives in `agentRuntime.ts`, which writes into this store via the actions
//! below — state commits are already coalesced per frame by the caller.

import { create } from "zustand";

import type { AgentState } from "./engineState";

export type AgentRuntimeState = {
  workstreamId: string;
  shellId: number;
  engineId: string;
  state: AgentState;
  /** Backend PTY id, once spawned/attached. Persisted onto the ShellTab too. */
  terminalId?: string;
  /** This run was (re)started in bypass-permissions mode (the red shield).
   *  Session-only — never persisted, so a normal relaunch clears it. */
  bypass?: boolean;
  /** A conversation has begun (first keystroke sent, or cold-started from a prior
   *  session). Gates the Restart/shield resume: without it `claude --continue` has
   *  nothing to continue and the CLI exits "No conversation found". Session-only. */
  started?: boolean;
};

type Store = { agents: Record<string, AgentRuntimeState> };

export const agentKey = (workstreamId: string, shellId: number): string =>
  `${workstreamId}:${shellId}`;

export const useMultiplexerStore = create<Store>(() => ({ agents: {} }));

/** Create or update an agent entry (preserving fields not in `patch`). */
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

/** Low-level store write for the bypass flag. Prefer `setAgentBypass` in
 *  agentRuntime, which keeps the runtime (the source of truth) in sync. */
export function setStoreBypass(workstreamId: string, shellId: number, bypass: boolean): void {
  const key = agentKey(workstreamId, shellId);
  useMultiplexerStore.setState((s) => {
    const cur = s.agents[key];
    if (!cur || !!cur.bypass === bypass) return s;
    return { agents: { ...s.agents, [key]: { ...cur, bypass } } };
  });
}

/** Mark that a conversation has begun in an agent (first keystroke / cold-start
 *  resume), so the Restart/shield can safely pass `--continue`. Idempotent and a
 *  no-op for an unknown agent, so calling it on every keystroke is cheap. */
export function markAgentStarted(workstreamId: string, shellId: number): void {
  const key = agentKey(workstreamId, shellId);
  useMultiplexerStore.setState((s) => {
    const cur = s.agents[key];
    if (!cur || cur.started) return s;
    return { agents: { ...s.agents, [key]: { ...cur, started: true } } };
  });
}

/** Commit a coalesced batch of state transitions (no-op for unknown agents). */
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

/** Subscribe to one agent's runtime state. */
export function useAgentState(
  workstreamId: string,
  shellId: number,
): AgentRuntimeState | undefined {
  return useMultiplexerStore((s) => s.agents[agentKey(workstreamId, shellId)]);
}

// Worst-first ordering: a blocked child dominates the workstream rollup.
const PRIORITY: Record<AgentState, number> = { blocked: 3, working: 2, done: 1, idle: 0 };

/** The highest-priority pty agent state across a workstream, or undefined if it
 *  has no pty agents (so the sidebar is unchanged when multiplexer is off). */
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

/** Reactive worst-state rollup for a workstream (sidebar dot). Selecting the
 *  derived state (not the whole `agents` map) lets zustand's Object.is bail out,
 *  so a dot only re-renders when its own rollup flips — not on every agent's
 *  per-frame output. */
export function useWorkstreamAgentRollup(workstreamId: string): AgentState | undefined {
  return useMultiplexerStore((s) => rollupForWorkstream(s.agents, workstreamId));
}
