import type { ReactNode } from "react";

import { cx } from "@/components/ui";
import type { WorkstreamView } from "@/lib/bindings";
import { useHostStats, useLeadQueue } from "@/lib/queries";
import { pipelineColumnOf, prNumberOf } from "@/features/board/pipeline";
import { HostMeters, hostRows } from "@/components/HostMeters";
import { useProjectLead } from "./lead";
import { LeadQueueList } from "./LeadQueue";

const STEPS = ["plan", "build", "validate", "merged"];

function reachedStep(view: WorkstreamView): number {
  switch (pipelineColumnOf(view, false)) {
    case "merged":
      return STEPS.length;
    case "review":
      return 2;
    case "implementing":
      return 1;
    default:
      return 0;
  }
}

function hasOpenPr(view: WorkstreamView): boolean {
  const observed = view.item.observedPr;
  return observed ? observed.state === "open" : view.item.prNumber !== null;
}

export function LeadPanel({
  projectPath,
  views,
  className,
}: {
  projectPath: string;
  views: WorkstreamView[];
  className?: string;
}) {
  const lead = useProjectLead(projectPath);
  const queue = useLeadQueue(projectPath).data ?? [];
  if (!lead) return null;

  const driven = new Set(lead.view.driving);
  const driving = views.filter((v) => driven.has(v.item.id));
  const others = views.filter((v) => v.item.id !== lead.view.workstreamId);
  const queued = queue.filter((item) => item.state === "queued");

  return (
    <div aria-label="Lead" className={cx("flex flex-col overflow-y-auto", className)}>
      <Section title="Queue" count={queued.length} tour="lead.panel.queue">
        {queued.length === 0 ? <Empty>Nothing queued</Empty> : <LeadQueueList projectPath={projectPath} items={queued} />}
      </Section>
      <Section title="Running" count={driving.length} tour="lead.panel.running">
        {driving.length === 0 ? (
          <Empty>No workstreams running</Empty>
        ) : (
          <ul>
            {driving.map((view) => (
              <DrivingRow key={view.item.id} view={view} />
            ))}
          </ul>
        )}
      </Section>
      <div data-tour="lead.panel.host">
        <Section title="Stats">
          <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 px-3 pb-3 text-meta">
            <Stat label="Agents" value={`${lead.view.liveChildren}/${lead.settings.maxLiveChildren}`} />
            <Stat label="Workstreams" value={String(others.length)} />
            <Stat label="Open PRs" value={String(others.filter(hasOpenPr).length)} />
          </dl>
        </Section>
        <HostSection />
      </div>
    </div>
  );
}

function Section({
  title,
  count,
  tour,
  children,
}: {
  title: string;
  count?: ReactNode;
  tour?: string;
  children: ReactNode;
}) {
  return (
    <section data-tour={tour} className="border-b border-edge last:border-b-0">
      <h3 className="flex h-9 items-center gap-2 px-3 text-ink-dim">
        <span className="text-label">{title}</span>
        {count !== undefined && <span className="ml-auto text-meta">{count}</span>}
      </h3>
      {children}
    </section>
  );
}

function Empty({ children }: { children: ReactNode }) {
  return <p className="px-3 pb-3 text-meta text-ink-4">{children}</p>;
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <>
      <dt className="text-ink-dim">{label}</dt>
      <dd className="text-right text-ink tabular-nums">{value}</dd>
    </>
  );
}

function HostSection() {
  const stats = useHostStats().data;
  if (!stats) return null;
  const cpu = stats.cpuPercent;
  return (
    <Section title="Host" count={`${stats.cpus} cores`}>
      {cpu == null ? (
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 px-3 pb-3 text-meta">
          {hostRows(stats).map(([label, value]) => (
            <Stat key={label} label={label} value={value} />
          ))}
        </dl>
      ) : (
        <HostMeters stats={stats} cpuPercent={cpu} />
      )}
    </Section>
  );
}

function DrivingRow({ view }: { view: WorkstreamView }) {
  const reached = reachedStep(view);
  const pr = prNumberOf(view);
  return (
    <li className="flex flex-col gap-2 border-t border-edge-2 px-3 py-3">
      <div className="flex items-center gap-2">
        <span className="min-w-0 flex-1 truncate text-body font-medium text-ink">{view.item.title}</span>
        {pr !== null && <span className="shrink-0 text-meta text-ink-4">#{pr}</span>}
      </div>
      <ol className="grid grid-cols-4 gap-[3px]">
        {STEPS.map((label, i) => (
          <li key={label} aria-current={i === reached ? "step" : undefined} className="flex flex-col gap-1">
            <span
              className={cx("h-[3px] rounded-[1px]", i < reached ? "bg-ok" : i === reached ? "bg-accent" : "bg-edge")}
            />
            <span className={cx("text-meta", i === reached ? "text-ink" : "text-ink-4")}>{label}</span>
          </li>
        ))}
      </ol>
    </li>
  );
}
