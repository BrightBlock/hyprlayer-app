import type { PtyAgentState } from "@/lib/domain/agentControl";
import type { AgentState } from "./engineState";

export function toAgentState(state: PtyAgentState, started: boolean): AgentState {
  switch (state) {
    case "working":
      return "working";
    case "awaiting_permission":
    case "awaiting_question":
      return "blocked";
    case "idle":
      return started ? "done" : "idle";
    case "starting":
    case "exited":
      return "idle";
  }
}
