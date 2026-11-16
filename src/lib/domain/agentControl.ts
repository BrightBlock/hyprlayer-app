import type {
  AgentControlLevel,
  ClientRef,
  PtyAgent,
  PtyAgentContext,
  PtyAgentState,
  PtyAgentStatus,
  PtyControlOutcome,
  PtyLaunched,
  PtyLaunchSession,
  SessionOrigin,
  UiRequest,
} from "@/lib/bindings";

export type {
  AgentControlLevel,
  PtyAgent,
  PtyAgentContext,
  PtyAgentState,
  PtyAgentStatus,
  PtyControlOutcome,
  PtyLaunched,
  PtyLaunchSession,
  SessionOrigin,
  UiRequest,
};

/** The server attributes an agent's turns to `agent:<caller id>`. */
export function isAgentAuthor(author: ClientRef | null | undefined): boolean {
  return !!author && author.clientId.startsWith("agent:");
}
