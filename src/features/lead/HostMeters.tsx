import type { ReactNode } from "react";

import { cx } from "@/components/ui";
import type { HostStats } from "@/lib/bindings";
import { formatBytes } from "@/lib/format";

const SPARK_SLOTS = 40;
const LOAD_TIP =
  "Processes running or waiting for a CPU, per core, averaged over the last minute. Under 1.0 means nothing is waiting.";

export type MeterTone = "ok" | "warn" | "bad";

/** The percent's colour; the bar itself takes its colour from the fixed gradient. */
export function meterTone(percent: number): MeterTone {
  return percent >= 90 ? "bad" : percent >= 70 ? "warn" : "ok";
}

export function formatPercent(percent: number): string {
  return percent > 0 && percent < 1 ? "<1%" : `${Math.round(percent)}%`;
}

/** Memory that is neither used nor free: file cache, which the kernel gives back on demand. */
export function cacheBytes(stats: HostStats): number | null {
  if (stats.memoryFree == null || !stats.memoryTotal) return null;
  return Math.max(0, stats.memoryTotal - (stats.memoryUsed ?? 0) - stats.memoryFree);
}

/** "11.9 of 62.4 GB", or "433 MB of 125 GB" when the units differ. */
export function ofTotal(used: number, total: number): string {
  const [usedValue, usedUnit] = formatBytes(used).split(" ");
  const totalText = formatBytes(total);
  return totalText.endsWith(` ${usedUnit}`) ? `${usedValue} of ${totalText}` : `${formatBytes(used)} of ${totalText}`;
}

export function hostRows(stats: HostStats): [string, string][] {
  const load = stats.load ? stats.load.map((n) => (n ?? 0).toFixed(2)).join(" ") : "n/a";
  const used = (a: number | null, b: number | null) => (b ? `${formatBytes(a ?? 0)} of ${formatBytes(b)}` : "none");
  return [
    ["Load", `${load} (${stats.cpus} cores)`],
    ["Memory", used(stats.memoryUsed, stats.memoryTotal)],
    ["Swap", used(stats.swapUsed, stats.swapTotal)],
  ];
}

const percentOf = (part: number, whole: number) => Math.min(100, Math.max(0, (part / whole) * 100));

const TONE_INK: Record<MeterTone, string> = {
  ok: "text-ink",
  warn: "text-meter-warn-ink",
  bad: "text-meter-bad-ink",
};
const TONE_FILL: Record<MeterTone, string> = { ok: "bg-meter-ok", warn: "bg-meter-warn", bad: "bg-meter-bad" };

export function HostMeters({ stats, cpuPercent }: { stats: HostStats; cpuPercent: number }) {
  const memTotal = stats.memoryTotal ?? 0;
  const memUsed = stats.memoryUsed ?? 0;
  const cache = cacheBytes(stats);
  const swapTotal = stats.swapTotal ?? 0;
  const loadPerCore = stats.load?.[0] != null ? (stats.load[0] / Math.max(1, stats.cpus)).toFixed(2) : null;

  return (
    <div className="flex flex-col gap-3.5 px-3 pb-4 font-mono tabular-nums">
      <div className="flex flex-col gap-1.5">
        <MeterRow label="CPU" ariaLabel="CPU usage" percent={cpuPercent} />
        <Sparkline samples={stats.cpuHistory ?? []} />
        <Caption>
          {loadPerCore !== null && <span title={LOAD_TIP}>load {loadPerCore}/core</span>}
          <span className="ml-auto pr-[46px]">last 2 min</span>
        </Caption>
      </div>

      {memTotal > 0 && (
        <div className="flex flex-col gap-1.5">
          <MeterRow
            label="Memory"
            ariaLabel="Memory used"
            percent={percentOf(memUsed, memTotal)}
            behind={cache === null ? undefined : Math.min(1, (memUsed + cache) / memTotal)}
          />
          <Caption>
            <span>{ofTotal(memUsed, memTotal)}</span>
            {cache !== null && (
              <span className="ml-auto inline-flex items-center gap-[5px] pr-[46px]">
                <span aria-hidden className="h-[7px] w-[7px] rounded-[1px] bg-meter-cache" />
                {formatBytes(cache)} cache
              </span>
            )}
          </Caption>
        </div>
      )}

      <div className="flex flex-col gap-1.5">
        {swapTotal > 0 ? (
          <>
            <MeterRow label="Swap" ariaLabel="Swap used" percent={percentOf(stats.swapUsed ?? 0, swapTotal)} />
            <Caption>
              <span>{ofTotal(stats.swapUsed ?? 0, swapTotal)}</span>
            </Caption>
          </>
        ) : (
          <div className={ROW}>
            <span className="text-meta text-ink-dim">Swap</span>
            <span className="col-span-2 text-meta text-ink-dim">none on this host</span>
          </div>
        )}
      </div>
    </div>
  );
}

const ROW = "grid grid-cols-[52px_minmax(0,1fr)_38px] items-center gap-x-2";

function MeterRow({
  label,
  ariaLabel,
  percent,
  behind,
}: {
  label: string;
  ariaLabel: string;
  percent: number;
  behind?: number;
}) {
  return (
    <div className={ROW}>
      <span className="text-meta text-ink-dim">{label}</span>
      <div
        role="meter"
        aria-label={ariaLabel}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(percent)}
        className="relative h-2 overflow-hidden rounded-[2px] bg-meter-track"
      >
        {behind !== undefined && (
          <div
            data-testid="cache-segment"
            className="host-meter-grow absolute inset-0 bg-meter-cache"
            style={{ transform: `scaleX(${behind})` }}
          />
        )}
        <div
          className="host-meter-reveal absolute inset-0 overflow-hidden"
          style={{ transform: `translateX(${percent - 100}%)` }}
        >
          <div
            className="host-meter-fill host-meter-unreveal absolute inset-0"
            style={{ transform: `translateX(${100 - percent}%)` }}
          />
        </div>
        <div className="host-meter-ticks absolute inset-0" />
      </div>
      <span className={cx("text-right font-mono text-ui-sm tabular-nums", TONE_INK[meterTone(percent)])}>
        {formatPercent(percent)}
      </span>
    </div>
  );
}

function Sparkline({ samples }: { samples: (number | null)[] }) {
  const recent = samples.slice(-SPARK_SLOTS);
  const slots: (number | null)[] = [...Array<null>(SPARK_SLOTS - recent.length).fill(null), ...recent];
  return (
    <div className="pr-[46px] pl-[60px]">
      <div role="img" aria-label="CPU usage over the last 2 minutes" className="flex h-[22px] items-end gap-px">
        {slots.map((v, i) => (
          <span
            key={i}
            className={cx("flex-1 rounded-t-[1px]", v !== null && TONE_FILL[meterTone(v)])}
            style={{ height: v === null ? 0 : `${Math.max(v, 3)}%` }}
          />
        ))}
      </div>
    </div>
  );
}

function Caption({ children }: { children: ReactNode }) {
  return <div className="flex items-center gap-2 pl-[60px] text-meta text-ink-dim">{children}</div>;
}
