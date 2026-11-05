import type { ArchivedWorkstreamView, Harness, WorkstreamView } from "@/lib/bindings";
import type { FleetActivity } from "@/features/dashboard/fleet";

export type PipelineKey = "planning" | "implementing" | "waiting" | "review" | "merged";

export type PipelineLive = { activity: FleetActivity; sessionId: string | null; line: string | null };

export type PipelineSession = { harness: Harness; model: string | null };

export const PIPELINE_COLUMNS: { key: PipelineKey; label: string; dot: string; count: string }[] = [
  { key: "planning", label: "Planning", dot: "bg-ink-4", count: "text-ink-4" },
  { key: "implementing", label: "Implementing", dot: "bg-accent", count: "text-accent" },
  { key: "waiting", label: "Waiting", dot: "bg-warn", count: "text-warn-ink" },
  { key: "review", label: "Review", dot: "bg-info", count: "text-info" },
  { key: "merged", label: "Merged", dot: "bg-ok", count: "text-ok" },
];

export type PipelineView = "triage" | "board" | "table";

export const PIPELINE_VIEWS: { value: PipelineView; label: string }[] = [
  { value: "triage", label: "Triage" },
  { value: "board", label: "Board" },
  { value: "table", label: "Table" },
];

export const MERGED_VISIBLE = 2;

// Untracked files don't count: a worktree's .envrc and thoughts/ would mark every one as started.
function hasBranchChanges(view: WorkstreamView): boolean {
  return view.git.dirty > 0 || view.git.aheadOfBase > 0;
}

// A live question outranks everything, then the PR, then the stage; a closed PR didn't ship.
export function pipelineColumnOf(view: WorkstreamView, waiting: boolean): PipelineKey {
  if (waiting) return "waiting";
  const pr = view.item.observedPr?.state;
  if (pr === "merged") return "merged";
  if (pr === "closed") return "implementing";
  if (pr === "open" || (pr === undefined && view.item.prNumber != null)) return "review";
  switch (view.item.stage) {
    case "implement":
      return "implementing";
    case "validate":
    case "commit":
    case "ship":
    case "revise":
      return "review";
    case "general":
      return hasBranchChanges(view) ? "implementing" : "planning";
    default:
      return "planning";
  }
}

export function prNumberOf(view: WorkstreamView): number | null {
  const observed = view.item.observedPr;
  if (observed && observed.state !== "closed") return observed.number;
  return view.item.prNumber;
}

export function columnDot(key: PipelineKey): string {
  return PIPELINE_COLUMNS.find((c) => c.key === key)?.dot ?? "bg-ink-4";
}

export function mergedToday(archived: ArchivedWorkstreamView[], now: number): ArchivedWorkstreamView[] {
  const start = new Date(now);
  start.setHours(0, 0, 0, 0);
  return archived
    .filter((v) => v.info.pr.state === "merged" && (v.item.archivedAt ?? 0) >= start.getTime())
    .sort((a, b) => (b.item.archivedAt ?? 0) - (a.item.archivedAt ?? 0));
}

export type TriageKey = "needs-you" | "error" | "review" | "running" | "idle";

export const TRIAGE_GROUPS: { key: TriageKey; label: string; dot: string }[] = [
  { key: "needs-you", label: "Needs you", dot: "bg-warn" },
  { key: "error", label: "Failed", dot: "bg-bad" },
  { key: "review", label: "Finished a turn", dot: "bg-info" },
  { key: "running", label: "Running", dot: "bg-accent" },
  { key: "idle", label: "Idle", dot: "bg-ink-4" },
];

export function triageKeyOf(live: PipelineLive | undefined): TriageKey {
  switch (live?.activity) {
    case "needs-you":
      return "needs-you";
    case "error":
      return "error";
    case "review":
      return "review";
    case "working":
    case "background":
      return "running";
    default:
      return "idle";
  }
}
