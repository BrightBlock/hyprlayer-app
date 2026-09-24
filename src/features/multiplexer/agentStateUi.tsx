
import type { AgentState } from "./engineState";
import { useAgentState } from "./multiplexerStore";

export const STATE_META: Record<AgentState, { label: string; token: string }> = {
  working: { label: "working", token: "--color-working" },
  blocked: { label: "blocked", token: "--color-warn" },
  done: { label: "done", token: "--color-ok" },
  idle: { label: "idle", token: "--color-ink-dim" },
};

export function AgentStateDot({ state, className }: { state: AgentState; className?: string }) {
  const meta = STATE_META[state];
  return (
    <span
      className={`inline-block h-1.5 w-1.5 shrink-0 rounded-full ${state === "working" ? "animate-work-pulse" : ""} ${className ?? ""}`}
      style={{ backgroundColor: `var(${meta.token})`, opacity: state === "idle" ? 0.5 : 1 }}
      aria-hidden="true"
    />
  );
}

export function PtyTabDot({ workstreamId, shellId }: { workstreamId: string; shellId: number }) {
  const agent = useAgentState(workstreamId, shellId);
  return <AgentStateDot state={agent?.state ?? "idle"} />;
}

export function AgentStatePill({ state }: { state: AgentState }) {
  const meta = STATE_META[state];
  return (
    <span className="inline-flex items-center gap-1.5" style={{ color: `var(${meta.token})` }}>
      <AgentStateDot state={state} />
      {meta.label}
    </span>
  );
}
