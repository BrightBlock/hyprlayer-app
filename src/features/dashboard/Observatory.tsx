import { Button, Kbd } from "@/components/ui";
import { Icon } from "@/components/Icon";
import type { ArchivedWorkstreamView, WorkstreamView } from "@/lib/bindings";
import { effectiveChord } from "@/lib/keybindings/resolve";
import { IS_MACOS } from "@/lib/platform";
import { useBootstrap } from "@/lib/queries";
import { MODE_STYLES, stageLabel, stageOf, type StageKey } from "@/lib/domain/stages";
import { formatElapsed, formatRelativeTime } from "@/lib/format";
import { openSearch } from "@/features/search/searchStore";
import type { FleetActivity, FleetItem } from "./fleet";

function shortElapsed(ms: number): string {
  if (ms < 60_000) return "<1m";
  const min = Math.floor(ms / 60_000);
  if (min < 60) return `${min}m`;
  return formatElapsed(ms);
}

const STAGE_DOT: Record<StageKey, string> = {
  general: "bg-working",
  research: "bg-research",
  plan: "bg-plan",
  implement: "bg-implement",
  validate: "bg-validate",
  commit: "bg-commit",
  ship: "bg-ship",
  revise: "bg-revise",
};

const STAGE_PIN_BORDER: Record<StageKey, string> = {
  general: "border-working/30",
  research: "border-research/30",
  plan: "border-plan/30",
  implement: "border-implement/30",
  validate: "border-validate/30",
  commit: "border-commit/30",
  ship: "border-ship/30",
  revise: "border-revise/30",
};

export function stageDot(view: WorkstreamView): string {
  return STAGE_DOT[stageOf(view)];
}

export function liveTone(item: FleetItem): { dot: string; pulse: boolean; border: string } {
  if (item.activity === "needs-you") return { dot: "bg-attention", pulse: true, border: "border-attention/30" };
  if (item.activity === "error") return { dot: "bg-bad", pulse: false, border: "border-bad/30" };
  if (item.activity === "review") return { dot: "bg-ok", pulse: false, border: "border-ok/30" };
  const stage = stageOf(item.view);
  return { dot: STAGE_DOT[stage], pulse: true, border: STAGE_PIN_BORDER[stage] };
}


type HeroStatus = { dot: string; pulse: boolean; text: string };

export function heroStatus(
  counts: { running: number; needsYou: number; review: number; errors: number; queued: number },
  opts: { hasProjects: boolean; loading: boolean; runningStages: string[] },
): HeroStatus {
  if (!opts.hasProjects) {
    return { dot: "bg-edge", pulse: false, text: "Add a repo to start dispatching agents" };
  }
  if (opts.loading) return { dot: "bg-edge", pulse: false, text: "Loading workstreams…" };
  const agents = (n: number) => `${n} agent${n === 1 ? "" : "s"}`;
  const tail: string[] = [];
  if (counts.errors > 0) tail.push(`${counts.errors} errored`);
  if (counts.review > 0) tail.push(`${counts.review} in review`);
  if (counts.queued > 0) tail.push(`${counts.queued} queued`);
  if (counts.needsYou > 0) {
    if (counts.running > 0) tail.unshift(`${counts.running} running`);
    return {
      dot: "bg-attention",
      pulse: true,
      text: [`${agents(counts.needsYou)} waiting on you`, ...tail].join(" · "),
    };
  }
  if (counts.running > 0) {
    const stages = opts.runningStages.join(" · ");
    return {
      dot: "bg-accent",
      pulse: true,
      text: [`${agents(counts.running)} running${stages ? `, ${stages}` : ""}`, ...tail].join(" · "),
    };
  }
  if (tail.length > 0) {
    return { dot: counts.errors > 0 ? "bg-bad" : "bg-ok", pulse: false, text: tail.join(" · ") };
  }
  return { dot: "bg-ok", pulse: false, text: "All quiet. No agents running" };
}


export function HeroSearch({ total }: { total: number }) {
  const bootstrap = useBootstrap();
  const chord = effectiveChord(
    bootstrap.data?.settings.keybindings?.overrides ?? {},
    "search.open",
    IS_MACOS,
  );
  return (
    <button
      type="button"
      onClick={openSearch}
      className="pointer-events-auto flex w-full max-w-[560px] items-center gap-3 rounded-[13px] border border-edge bg-panel/70 px-[18px] py-3.5 text-left shadow-[0_12px_40px_rgba(0,0,0,0.35)] backdrop-blur-xl hover:border-ink-dim/50"
    >
      <span className="flex text-ink-dim">
        <Icon name="search" />
      </span>
      <span className="min-w-0 flex-1 truncate text-body text-ink-dim">
        Search {total > 0 ? `${total} ${total === 1 ? "document" : "documents"}, ` : ""}
        workstreams, agents…
      </span>
      <Kbd chord={chord} className="shrink-0" />
    </button>
  );
}

function StatPill({ n, label }: { n: number; label: string }) {
  return (
    <span className="rounded-full border border-edge bg-panel/60 px-3 py-1 text-ui-sm text-ink-dim backdrop-blur-md">
      <b className="font-semibold text-ink">{n}</b> {label}
    </span>
  );
}

export function StatPills({
  documents,
  links,
  projects,
  agentsLive,
  weekCount,
  hint,
}: {
  documents: number;
  links: number;
  projects: number;
  agentsLive: number;
  weekCount: number;
  hint?: string;
}) {
  return (
    <div
      className="pointer-events-auto flex flex-wrap items-center justify-center gap-2"
      title={hint}
    >
      <StatPill n={documents} label={documents === 1 ? "document" : "documents"} />
      <StatPill n={links} label={links === 1 ? "link" : "links"} />
      <StatPill n={projects} label={projects === 1 ? "project" : "projects"} />
      {agentsLive > 0 ? (
        <span className="rounded-full border border-accent/30 bg-accent/10 px-3 py-1 text-ui-sm text-accent backdrop-blur-md">
          <b className="font-semibold">{agentsLive}</b> {agentsLive === 1 ? "agent" : "agents"} live
        </span>
      ) : (
        weekCount > 0 && (
          <StatPill n={weekCount} label={`workstream${weekCount === 1 ? "" : "s"} this week`} />
        )
      )}
    </div>
  );
}


export function SetupPill({ onClick }: { onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="pointer-events-auto absolute bottom-6 right-6 flex items-center gap-2 rounded-full border border-edge bg-panel/75 px-3.5 py-2 backdrop-blur-md hover:border-ink-dim/40"
    >
      <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />
      <span className="text-ui font-medium text-ink">Get set up</span>
      <span className="flex text-ink-dim">
        <Icon name="chevronRight" size="sm" />
      </span>
    </button>
  );
}


export type GraphPin = {
  key: string;
  label: string;
  dot: string;
  pulse: boolean;
  border?: string;
  onClick: () => void;
};

const PIN_SLOTS = [
  { left: "57%", top: "26%" },
  { left: "27%", top: "50%" },
  { left: "66%", top: "47%" },
] as const;

export function GraphPins({ pins }: { pins: GraphPin[] }) {
  if (pins.length === 0) return null;
  return (
    <div className="pointer-events-none absolute inset-0 hidden lg:block">
      {pins.slice(0, PIN_SLOTS.length).map((pin, i) => (
        <button
          key={pin.key}
          type="button"
          onClick={pin.onClick}
          style={PIN_SLOTS[i]}
          className={`pointer-events-auto absolute flex -translate-y-1/2 items-center gap-1.5 rounded-full border ${
            pin.border ?? "border-edge"
          } bg-panel/70 px-2.5 py-1 text-ui-sm text-ink-dim backdrop-blur-md hover:text-ink`}
        >
          <span
            className={`h-1.5 w-1.5 shrink-0 rounded-full ${pin.dot} ${pin.pulse ? "animate-pulse" : ""}`}
          />
          <span className="max-w-[240px] truncate">{pin.label}</span>
        </button>
      ))}
    </div>
  );
}


const GLASS_PANEL =
  "flex min-h-[150px] min-w-0 flex-col rounded-[14px] border border-edge bg-panel/75 p-4 shadow-[0_16px_48px_rgba(0,0,0,0.35)] backdrop-blur-xl";

function PanelLabel({ children }: { children: string }) {
  return (
    <span className="text-label text-ink-dim">
      {children}
    </span>
  );
}

const STATE_PILL: Partial<Record<FleetActivity, { label: string; tone: string }>> = {
  "needs-you": { label: "needs you", tone: "bg-attention/15 text-attention" },
  error: { label: "error", tone: "bg-bad/15 text-bad" },
  review: { label: "review", tone: "bg-ok/15 text-ok" },
};

function AgentCard({
  item,
  project,
  now,
  onOpen,
}: {
  item: FleetItem;
  project: string;
  now: number;
  onOpen: (item: FleetItem) => void;
}) {
  const { view, activity, line, meter, since, queued } = item;
  const running = activity === "working" || activity === "background";
  const stage = stageOf(view);
  const tone = liveTone(item);
  const git = view.git;
  const statePill = STATE_PILL[activity];
  return (
    <button
      type="button"
      onClick={() => onOpen(item)}
      className="flex w-full shrink-0 flex-col gap-2 rounded-[10px] border border-edge/70 bg-panel-2/40 px-3 py-2.5 text-left hover:border-ink-dim/40"
    >
      <span className="flex w-full items-center gap-2.5">
        <span
          className={`h-2 w-2 shrink-0 rounded-full ${tone.dot} ${tone.pulse ? "animate-pulse" : ""}`}
        />
        <span className="max-w-[45%] shrink-0 truncate text-body font-semibold text-ink">
          {view.item.title}
        </span>
        <span className="min-w-0 flex-1 truncate text-meta text-ink-dim">
          {activity === "review" && git.dirty > 0 ? (
            <>
              <span className="text-diff-add">+{git.additions}</span>{" "}
              <span className="text-diff-del">−{git.deletions}</span> · {git.dirty}{" "}
              {git.dirty === 1 ? "file" : "files"}
            </>
          ) : (
            <>
              {project} ·{" "}
              {line ??
                (activity === "review" ? "turn finished, ready for review" : view.item.branch)}
              {queued > 0 && ` · +${queued} queued`}
            </>
          )}
        </span>
        {running ? (
          <span
            className={`shrink-0 rounded-full px-2 py-px text-ui-sm font-semibold ${MODE_STYLES[stage].badge}`}
          >
            {stageLabel(stage).toLowerCase()}
          </span>
        ) : (
          statePill && (
            <span
              className={`shrink-0 rounded-full px-2 py-px text-ui-sm font-semibold ${statePill.tone}`}
            >
              {statePill.label}
            </span>
          )
        )}
        {running && since !== null && (
          <span className="w-9 shrink-0 text-right text-meta text-ink-dim">
            {shortElapsed(now - since)}
          </span>
        )}
      </span>
      {running && meter && (
        <span className="relative block h-[3px] w-full overflow-hidden rounded-full bg-panel-2/80">
          <span
            className={`absolute inset-y-0 left-0 rounded-full ${STAGE_DOT[stage]}`}
            style={{ width: `${Math.max(2, Math.round(meter.fraction * 100))}%` }}
          />
        </span>
      )}
    </button>
  );
}

export function LiveNowPanel({
  items,
  now,
  projectName,
  onOpen,
  emptyHint,
  lastShipped,
  onStartAgent,
  onOpenLibrary,
}: {
  items: FleetItem[];
  now: number;
  projectName: (path: string) => string;
  onOpen: (item: FleetItem) => void;
  emptyHint: string;
  lastShipped: { title: string; pr: number | null; at: number } | null;
  onStartAgent: (() => void) | null;
  onOpenLibrary: () => void;
}) {
  return (
    <section className={GLASS_PANEL}>
      <span className="flex items-center gap-2.5">
        <PanelLabel>Live now</PanelLabel>
        {items.length > 0 && (
          <span className="rounded-full border border-accent/25 bg-accent/10 px-2 py-px text-ui-sm font-semibold text-accent">
            {items.length} live
          </span>
        )}
      </span>
      {items.length === 0 ? (
        <div className="flex flex-1 flex-col justify-center gap-3 py-2">
          <p className="text-body leading-prose text-ink-dim">
            {emptyHint}
            {lastShipped && (
              <>
                {" "}
                Your last agent shipped{" "}
                <b className="font-semibold text-ink">{lastShipped.title}</b>
                {lastShipped.pr !== null && ` (PR #${lastShipped.pr})`}{" "}
                {formatRelativeTime(lastShipped.at, now)}.
              </>
            )}
          </p>
          <span className="flex flex-wrap gap-2.5">
            {onStartAgent && (
              <Button variant="primary" onClick={onStartAgent} className="text-ui">
                Start an agent
              </Button>
            )}
            <Button onClick={onOpenLibrary} className="text-ui">
              Open library
            </Button>
          </span>
        </div>
      ) : (
        <div className="mt-3 flex max-h-[170px] flex-col gap-2 overflow-y-auto">
          {items.map((item) => (
            <AgentCard
              key={item.view.item.id}
              item={item}
              project={projectName(item.view.item.projectPath)}
              now={now}
              onOpen={onOpen}
            />
          ))}
        </div>
      )}
    </section>
  );
}


export type RecentRow =
  | { kind: "active"; at: number; view: WorkstreamView; project: string }
  | { kind: "archived"; at: number; view: ArchivedWorkstreamView; project: string; merged: boolean };

function rowDot(row: RecentRow): string {
  if (row.kind === "archived") return row.merged ? "bg-ok" : "bg-edge";
  return stageDot(row.view);
}

export function ActivityPanel({
  rows,
  now,
  onOpenActive,
  onOpenArchive,
}: {
  rows: RecentRow[];
  now: number;
  onOpenActive: (view: WorkstreamView) => void;
  onOpenArchive: () => void;
}) {
  return (
    <section className={GLASS_PANEL}>
      <span className="flex items-baseline">
        <PanelLabel>Latest activity</PanelLabel>
        <span className="flex-1" />
        <button
          type="button"
          onClick={onOpenArchive}
          className="text-ui-sm text-ink-dim hover:text-ink"
        >
          View all →
        </button>
      </span>
      <div className="mt-2 flex max-h-[170px] flex-col gap-0.5 overflow-y-auto">
        {rows.length === 0 && (
          <p className="px-1.5 py-2 text-ui text-ink-dim">No activity yet.</p>
        )}
        {rows.map((row) => (
          <button
            key={`${row.kind}-${row.view.item.id}`}
            type="button"
            onClick={() => (row.kind === "active" ? onOpenActive(row.view) : onOpenArchive())}
            className="flex w-full shrink-0 items-center gap-2.5 rounded-lg px-1.5 py-1 text-left hover:bg-edge/50"
          >
            <span className={`h-1.5 w-1.5 shrink-0 rounded-full ${rowDot(row)}`} />
            <span className="min-w-0 truncate text-ui font-semibold text-ink">
              {row.view.item.title}
            </span>
            <span className="shrink-0 text-meta text-ink-dim">{row.project}</span>
            <span className="min-w-0 flex-1" />
            {row.kind === "archived" && (
              <span
                className={`shrink-0 text-meta ${row.merged ? "text-ok" : "text-ink-dim"}`}
              >
                {row.merged ? "merged" : "archived"}
              </span>
            )}
            <span className="w-8 shrink-0 text-right text-meta text-ink-dim">
              {formatRelativeTime(row.at, now).replace(" ago", "")}
            </span>
          </button>
        ))}
      </div>
    </section>
  );
}
