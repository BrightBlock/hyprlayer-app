import { Icon } from "@/components/Icon";
import { cx } from "@/components/ui";
import type { WorkstreamView } from "@/lib/bindings";
import { plural } from "@/lib/format";
import { useUiStore } from "@/lib/uiStore";
import { pipelineColumnOf, prNumberOf } from "@/features/board/pipeline";
import { useProjectLead } from "./lead";

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

export function LeadDrivingPanel({
  projectPath,
  workstreamId,
  views,
}: {
  projectPath: string;
  workstreamId: string | null;
  views: WorkstreamView[];
}) {
  const lead = useProjectLead(projectPath);
  const collapsed = useUiStore((s) => s.drivingCollapsed);
  const toggle = useUiStore((s) => s.toggleDriving);

  if (!lead || !workstreamId || workstreamId !== lead.view.workstreamId) return null;
  const driven = new Set(lead.view.driving);
  const driving = views.filter((v) => driven.has(v.item.id));
  if (driving.length === 0) return null;

  const open = !collapsed;
  const agents = `agents ${lead.view.liveChildren}/${lead.settings.maxLiveChildren}`;
  const summary = open ? agents : `${plural(driving.length, "workstream")} · ${agents}`;

  return (
    <div className={cx("flex flex-col border-b border-edge", open ? "max-h-[50%] min-h-0 shrink" : "shrink-0")}>
      <button
        type="button"
        onClick={toggle}
        aria-label={`${open ? "Hide" : "Show"} driving, ${summary}`}
        aria-expanded={open}
        className="flex h-9 shrink-0 items-center gap-2 px-3 text-left text-ink-dim hover:text-ink"
      >
        <span className="flex w-3 shrink-0 justify-center">
          <Icon name={open ? "chevronDown" : "chevronRight"} size="xs" />
        </span>
        <span className="text-label">Driving</span>
        <span className="ml-auto text-meta">{summary}</span>
      </button>
      {open && (
        <ul className="min-h-0 overflow-y-auto border-t border-edge">
          {driving.map((view) => (
            <DrivingRow key={view.item.id} view={view} />
          ))}
        </ul>
      )}
    </div>
  );
}

function DrivingRow({ view }: { view: WorkstreamView }) {
  const reached = reachedStep(view);
  const pr = prNumberOf(view);
  return (
    <li className="flex flex-col gap-2 border-b border-edge-2 px-3 py-3 last:border-b-0">
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
