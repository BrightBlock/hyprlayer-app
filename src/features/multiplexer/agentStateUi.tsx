//! Shared visuals for an agent's semantic state — the tab-strip dot, the status
//! bar pill, and the engine glyph. All colors come from theme tokens (no hex).

import type { AgentState } from "./engineState";
import { useAgentState } from "./multiplexerStore";

export const STATE_META: Record<AgentState, { label: string; token: string }> = {
  working: { label: "working", token: "--color-working" },
  blocked: { label: "blocked", token: "--color-warn" },
  done: { label: "done", token: "--color-ok" },
  idle: { label: "idle", token: "--color-ink-dim" },
};

/** A small state dot: working breathes; idle is muted; others are solid. */
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

/** The tab-strip dot for a pty agent: subscribes to its live state from the
 *  store (its own component so the parent pill loop doesn't call a hook). */
export function PtyTabDot({ workstreamId, shellId }: { workstreamId: string; shellId: number }) {
  const agent = useAgentState(workstreamId, shellId);
  return <AgentStateDot state={agent?.state ?? "idle"} />;
}

/** Dot + label, tinted by state — for the agent status bar. */
export function AgentStatePill({ state }: { state: AgentState }) {
  const meta = STATE_META[state];
  return (
    <span className="inline-flex items-center gap-1.5" style={{ color: `var(${meta.token})` }}>
      <AgentStateDot state={state} />
      {meta.label}
    </span>
  );
}

/** An engine's single-glyph mark, tinted by its accent token. */
export function EngineGlyph({
  glyph,
  accentToken,
  className,
}: {
  glyph: string;
  accentToken: string;
  className?: string;
}) {
  return (
    <span className={className} style={{ color: `var(${accentToken})` }} aria-hidden="true">
      {glyph}
    </span>
  );
}
