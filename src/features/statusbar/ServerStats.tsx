import type { ReactNode } from "react";

import { formatPercent, meterTone, TONE_FILL } from "@/components/HostMeters";
import { Icon, type IconName } from "@/components/Icon";
import { cx } from "@/components/ui";
import type { HostStats } from "@/lib/bindings";
import { useActiveSessions, useAllWorkstreams, useHostStats } from "@/lib/queries";

const SPARK_SLOTS = 6;
const SWAP_SHOWN_FROM = 10;

export type CpuReading =
  | { kind: "pending" }
  | { kind: "percent"; percent: number; recent: (number | null)[] }
  | { kind: "load"; perCore: number | null };

export type ServerStatsView = {
  cpu: CpuReading;
  memory: number | null;
  /** `null` until swap use reaches `SWAP_SHOWN_FROM` percent. */
  swap: number | null;
  agents: number | null;
  workstreams: number | null;
};

const percentOf = (part: number, whole: number) => Math.min(100, Math.max(0, (part / whole) * 100));

function cpuReading(stats: HostStats | undefined): CpuReading {
  if (!stats) return { kind: "pending" };
  // A current server sends a history from its first sample; an older one sends none.
  if (stats.cpuHistory == null) {
    const load = stats.load?.[0];
    return { kind: "load", perCore: load == null ? null : load / Math.max(1, stats.cpus) };
  }
  if (stats.cpuPercent == null) return { kind: "pending" };
  const recent = stats.cpuHistory.slice(-SPARK_SLOTS);
  return {
    kind: "percent",
    percent: stats.cpuPercent,
    recent: [...Array<null>(SPARK_SLOTS - recent.length).fill(null), ...recent],
  };
}

export function serverStatsView(
  stats: HostStats | undefined,
  agents: number | null,
  workstreams: number | null,
): ServerStatsView {
  const memory = stats?.memoryTotal ? percentOf(stats.memoryUsed ?? 0, stats.memoryTotal) : null;
  const swap = stats?.swapTotal ? percentOf(stats.swapUsed ?? 0, stats.swapTotal) : null;
  return {
    cpu: cpuReading(stats),
    memory,
    swap: swap !== null && swap >= SWAP_SHOWN_FROM ? swap : null,
    agents,
    workstreams,
  };
}

const plural = (n: number, noun: string) => `${n} ${n === 1 ? noun : `${noun}s`}`;

/** The numbers the readout leaves out, for its tooltip and accessible name. */
export function serverStatsSummary(view: ServerStatsView): string {
  const parts: string[] = [];
  if (view.cpu.kind === "percent") parts.push(`CPU ${formatPercent(view.cpu.percent)}`);
  if (view.cpu.kind === "load" && view.cpu.perCore !== null) parts.push(`load ${view.cpu.perCore.toFixed(2)}/core`);
  if (view.memory !== null) parts.push(`Memory ${formatPercent(view.memory)}`);
  if (view.swap !== null) parts.push(`Swap ${formatPercent(view.swap)}`);
  if (view.agents !== null) parts.push(`${plural(view.agents, "agent")} running`);
  if (view.workstreams !== null) parts.push(plural(view.workstreams, "workstream"));
  return parts.join(" · ");
}

/** Polls only while `enabled`; `null` when off. */
export function useServerStats(enabled: boolean): ServerStatsView | null {
  const stats = useHostStats(enabled).data;
  const agents = useActiveSessions(enabled).data?.length ?? null;
  const workstreams = useAllWorkstreams(enabled).data?.length ?? null;
  return enabled ? serverStatsView(stats, agents, workstreams) : null;
}

/** Graphs and counts with no percentages; those live in the tooltip and the Server menu. */
export function ServerStatsReadout({ view, compact }: { view: ServerStatsView; compact: boolean }) {
  const counts = !compact && (view.agents !== null || view.workstreams !== null);
  return (
    <span className="flex items-center gap-2.5">
      <Rule />
      <Part icon={view.cpu.kind === "load" ? "load" : "cpu"}>
        <Cpu reading={view.cpu} />
      </Part>
      <Part icon="memory">
        <Meter testId="stats-memory" percent={view.memory} />
      </Part>
      {view.swap !== null && (
        <Part icon="swap">
          <Meter testId="stats-swap" percent={view.swap} />
        </Part>
      )}
      {counts && (
        <>
          <Rule />
          {view.agents !== null && <Count testId="stats-agents" icon="agents" value={view.agents} />}
          {view.workstreams !== null && <Count testId="stats-workstreams" icon="branch" value={view.workstreams} />}
        </>
      )}
    </span>
  );
}

function Rule() {
  return <span aria-hidden="true" className="h-3 w-px shrink-0 bg-edge" />;
}

function Part({ icon, children }: { icon: IconName; children: ReactNode }) {
  return (
    <span className="flex items-center gap-[5px]">
      <Icon name={icon} size="xs" className="text-ink-dim" />
      {children}
    </span>
  );
}

const GRAPH = "relative flex h-3 w-[35px] shrink-0";

function Cpu({ reading }: { reading: CpuReading }) {
  if (reading.kind === "load") {
    return (
      <span data-testid="stats-load" className="font-mono text-meta tabular-nums text-ink-2">
        {reading.perCore === null ? "–" : reading.perCore.toFixed(2)}
      </span>
    );
  }
  const slots = reading.kind === "percent" ? reading.recent : Array<null>(SPARK_SLOTS).fill(null);
  return (
    <span
      data-testid={reading.kind === "percent" ? "stats-cpu" : "stats-cpu-pending"}
      aria-hidden="true"
      className={cx(GRAPH, "items-end gap-px")}
    >
      {slots.map((v, i) =>
        v === null ? (
          <span key={i} className={cx("flex-1", reading.kind === "pending" && "h-px bg-ink-4")} />
        ) : (
          <span
            key={i}
            data-tone={meterTone(v)}
            className={cx("flex-1", TONE_FILL[meterTone(v)])}
            style={{ height: `${Math.max(v, 7)}%` }}
          />
        ),
      )}
    </span>
  );
}

function Meter({ percent, testId }: { percent: number | null; testId: string }) {
  return (
    <span
      data-testid={testId}
      data-percent={percent === null ? undefined : Math.round(percent)}
      aria-hidden="true"
      className={cx(GRAPH, "overflow-hidden rounded-[2px] bg-meter-track")}
    >
      {percent !== null && (
        <span className="absolute inset-y-0 left-0 overflow-hidden" style={{ width: `${percent}%` }}>
          <span className="host-meter-fill absolute inset-y-0 left-0 w-[35px]" />
        </span>
      )}
      <span className="host-meter-ticks absolute inset-0" />
    </span>
  );
}

function Count({ icon, value, testId }: { icon: IconName; value: number; testId: string }) {
  return (
    <span data-testid={testId} className="flex items-center gap-[5px]">
      <Icon name={icon} size="xs" className="text-ink-dim" />
      <span className="font-mono text-meta tabular-nums text-ink-2">{value}</span>
    </span>
  );
}
