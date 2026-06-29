import { describe, expect, it } from "vitest";

import type { AgentState } from "./engineState";
import { rollupForWorkstream, type AgentRuntimeState } from "./multiplexerStore";

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
