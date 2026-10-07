import type { ConnectionStatus, WorkstreamView } from "@/lib/bindings";
import { isDetachedHead, isFolderWorkstream } from "@/lib/domain/projects";
import { formatCompactAge } from "@/lib/format";
import type { ReconnectHalt, ReconnectSnapshot } from "@/features/hosts/reconnectState";

export type RepoSummary = { branch: string; additions: number; deletions: number };

// TODO(statusbar): a line count against the base branch; only uncommitted-vs-HEAD exists.
export function repoSummary(view: WorkstreamView): RepoSummary | null {
  if (isFolderWorkstream(view) || !view.git.exists) return null;
  return {
    branch: isDetachedHead(view) ? "detached" : (view.git.branch ?? view.item.branch),
    additions: view.git.additions,
    deletions: view.git.deletions,
  };
}

export type MeterState = "normal" | "warn" | "full";

export function meterPercent(tokens: number, window: number): number | null {
  if (!Number.isFinite(tokens) || !Number.isFinite(window) || window <= 0) return null;
  return Math.min(100, Math.max(0, Math.floor((100 * tokens) / window)));
}

export function meterState(pct: number): MeterState {
  if (pct >= 100) return "full";
  if (pct >= 90) return "warn";
  return "normal";
}

// A usage limit is not an error, so no state takes `bad`.
const METER_CLASSES: Record<MeterState, { fill: string; text: string }> = {
  normal: { fill: "bg-ink-dim", text: "font-normal text-ink-dim" },
  warn: { fill: "bg-warn", text: "font-semibold text-ink" },
  full: { fill: "bg-ink", text: "font-semibold text-ink" },
};

export function meterClasses(state: MeterState): { fill: string; text: string } {
  return METER_CLASSES[state];
}

export type SyncTone = "ok" | "warn" | "bad";

export type SyncLabel = { text: string; title: string; tone: SyncTone; ticking: boolean };

const connected = (named: string | null, title: string, tone: SyncTone): SyncLabel => ({
  text: named ? `connected · ${named}` : "connected",
  title,
  tone,
  ticking: false,
});

function sinceDrop(lostAt: number, now: number): string {
  const ms = Math.max(0, now - lostAt);
  return ms < 60_000 ? `${Math.floor(ms / 1000)}s` : formatCompactAge(lostAt, now);
}

// TODO(statusbar): latency needs a ping RPC; `Ping` exists only under #[cfg(test)] in crates/hyprlayer-proto/src/commands.rs.
// TODO(statusbar): a `paused` state needs something that pauses sync; nothing does.
const HALT_REASON: Record<ReconnectHalt, string> = {
  protocol: "protocol mismatch",
  signIn: "sign-in cancelled",
};

export function syncLabel(
  status: ConnectionStatus | null,
  r: ReconnectSnapshot,
  now: number,
): SyncLabel | null {
  if (!status) return null;
  const named = status.host
    ? status.host.label || status.host.destination
    : (status.localServerName ?? null);
  const label = named ?? "this machine";

  if (status.state === "connected") {
    if (status.serverOutdated) return connected(named, `Server update available on ${label}`, "warn");
    return connected(named, `Server: ${label}`, "ok");
  }

  const failed = status.state === "failed";
  const tone: SyncTone = failed ? "bad" : "warn";
  const title = failed
    ? `Not connected to ${label}`
    : `Connection to ${label} lost${r.halted ? ` · stopped retrying (${HALT_REASON[r.halted]})` : ""}`;
  if (r.attempting && !r.halted) return { text: "retrying", title, tone, ticking: false };
  // A launch that never connected dropped nothing, so there is no drop to count from.
  if (failed || r.lostAt === null) return { text: "offline", title, tone, ticking: false };
  return { text: `offline · ${sinceDrop(r.lostAt, now)}`, title, tone, ticking: true };
}
