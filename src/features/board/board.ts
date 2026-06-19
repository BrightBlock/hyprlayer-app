import type { WorkstreamView } from "../../lib/bindings";
import type { StageKey } from "../session/stages";

export type ColumnKey = "backlog" | "research" | "plan" | "implement" | "validate" | "ship";

/** Columns left→right along the spec-driven pipeline. `stageKey` selects the
 *  shared hue + icon (MODE_STYLES / ModeIcon) so a column reads the same as the
 *  chat tab for that stage. */
export const BOARD_COLUMNS: { key: ColumnKey; label: string; stageKey: StageKey }[] = [
  { key: "backlog", label: "Backlog", stageKey: "general" },
  { key: "research", label: "Research", stageKey: "research" },
  { key: "plan", label: "Plan", stageKey: "plan" },
  { key: "implement", label: "Implement", stageKey: "implement" },
  { key: "validate", label: "Validate", stageKey: "validate" },
  { key: "ship", label: "Ship", stageKey: "ship" },
];

/** Which column a workstream belongs in. A live PR trumps the chat stage —
 *  once a PR exists the work is shipping/in-review regardless of the last chat
 *  mode. Otherwise the persisted `stage` maps 1:1, with the header/PR-driven
 *  stages (commit/ship/revise) collecting in Ship and anything unknown in
 *  Backlog. */
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

/** Bucket active workstreams into columns, each sorted newest-updated first. */
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
