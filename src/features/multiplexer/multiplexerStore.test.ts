import { afterEach, describe, expect, it } from "vitest";

import type { AgentState } from "./engineState";
import {
  agentKey,
  markAgentStarted,
  rollupForWorkstream,
  setAgentStateBatch,
  upsertAgent,
  useMultiplexerStore,
  type AgentRuntimeState,
} from "./multiplexerStore";

const agent = (workstreamId: string, shellId: number, state: AgentState): AgentRuntimeState => ({
  workstreamId,
  shellId,
  engineId: "claude-code",
  state,
});

describe("rollupForWorkstream", () => {
  const agents: Record<string, AgentRuntimeState> = {
    "ws1:1": agent("ws1", 1, "done"),
    "ws1:2": agent("ws1", 2, "blocked"),
    "ws1:3": agent("ws1", 3, "working"),
    "ws2:1": agent("ws2", 1, "working"),
  };

  it("returns the worst (highest-priority) state across a workstream", () => {
    expect(rollupForWorkstream(agents, "ws1")).toBe("blocked");
    expect(rollupForWorkstream(agents, "ws2")).toBe("working");
  });

  it("returns undefined for a workstream with no pty agents", () => {
    // Keeps the sidebar unchanged when multiplexer is off / no agents.
    expect(rollupForWorkstream(agents, "ws3")).toBeUndefined();
    expect(rollupForWorkstream({}, "ws1")).toBeUndefined();
  });
});

describe("markAgentStarted", () => {
  afterEach(() => useMultiplexerStore.setState({ agents: {} }));

  const startedOf = (ws: string, shell: number) =>
    useMultiplexerStore.getState().agents[agentKey(ws, shell)]?.started;

  it("seeds started=false on creation and flips it true on first keystroke", () => {
    // The reported bug: a fresh tab must NOT resume (`claude --continue` → "No
    // conversation found") until the user has actually typed something.
    upsertAgent("ws", 1, { engineId: "claude-code", state: "idle", started: false });
    expect(startedOf("ws", 1)).toBe(false);

    markAgentStarted("ws", 1);
    expect(startedOf("ws", 1)).toBe(true);
  });

  it("is idempotent and a no-op for an unknown agent", () => {
    upsertAgent("ws", 1, { engineId: "claude-code", state: "idle", started: false });
    markAgentStarted("ws", 1);
    const ref = useMultiplexerStore.getState().agents;
    markAgentStarted("ws", 1); // already started → same object reference (no churn)
    expect(useMultiplexerStore.getState().agents).toBe(ref);
    markAgentStarted("ws", 99); // unknown → no-op
    expect(startedOf("ws", 99)).toBeUndefined();
  });

  it("survives a state-only update so detection flushes don't reset resume", () => {
    // setAgentStateBatch (the per-frame working/blocked/done write) must preserve
    // `started`, else the shield would lose its resume mid-conversation.
    upsertAgent("ws", 1, { engineId: "claude-code", state: "idle", started: false });
    markAgentStarted("ws", 1);
    setAgentStateBatch(new Map([[agentKey("ws", 1), "working" as AgentState]]));
    expect(startedOf("ws", 1)).toBe(true);
  });
});
