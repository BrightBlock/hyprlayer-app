import type { WorkstreamView } from "@/lib/bindings";
import type { StageKey } from "@/lib/domain/stages";

export type ColumnKey = "backlog" | "research" | "plan" | "implement" | "validate" | "ship";

export const BOARD_COLUMNS: { key: ColumnKey; label: string; stageKey: StageKey }[] = [
  { key: "backlog", label: "Backlog", stageKey: "general" },
  { key: "research", label: "Research", stageKey: "research" },
  { key: "plan", label: "Plan", stageKey: "plan" },
  { key: "implement", label: "Implement", stageKey: "implement" },
  { key: "validate", label: "Validate", stageKey: "validate" },
  { key: "ship", label: "Ship", stageKey: "ship" },
];

export function columnOf(view: WorkstreamView): ColumnKey {
  if (view.item.prNumber != null) return "ship";
  switch (view.item.stage) {
    case "research":
      return "research";
    case "plan":
      return "plan";
    case "implement":
      return "implement";
    case "validate":
      return "validate";
    case "commit":
    case "ship":
    case "revise":
      return "ship";
    case "general":
    default:
      return "backlog";
  }
}

export function groupByColumn(views: WorkstreamView[]): Record<ColumnKey, WorkstreamView[]> {
  const out: Record<ColumnKey, WorkstreamView[]> = {
    backlog: [],
    research: [],
    plan: [],
    implement: [],
    validate: [],
    ship: [],
  };
  for (const v of views) out[columnOf(v)].push(v);
  for (const key of Object.keys(out) as ColumnKey[]) {
    out[key].sort((a, b) => (b.item.updatedAt ?? 0) - (a.item.updatedAt ?? 0));
  }
  return out;
}
