
import type { AgentState } from "./engineState";
import { useAgentState } from "./multiplexerStore";

const STATE_TOKEN: Record<AgentState, string> = {
  working: "--color-working",
  blocked: "--color-warn",
  done: "--color-ok",
  idle: "--color-ink-dim",
};

export function AgentStateDot({ state, className }: { state: AgentState; className?: string }) {
  return (
    <span
      className={`inline-block h-1.5 w-1.5 shrink-0 rounded-full ${state === "working" ? "animate-work-pulse" : ""} ${className ?? ""}`}
      style={{ backgroundColor: `var(${STATE_TOKEN[state]})`, opacity: state === "idle" ? 0.5 : 1 }}
      aria-hidden="true"
    />
  );
}

export function PtyTabDot({ workstreamId, shellId }: { workstreamId: string; shellId: number }) {
  const agent = useAgentState(workstreamId, shellId);
  return <AgentStateDot state={agent?.state ?? "idle"} />;
}
