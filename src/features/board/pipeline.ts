import type { ArchivedWorkstreamView, WorkstreamView } from "@/lib/bindings";

export type PipelineKey = "planning" | "implementing" | "waiting" | "review" | "merged";

export type PipelineWaiting = { sessionId: string | null; line: string | null };

export const PIPELINE_COLUMNS: { key: PipelineKey; label: string; dot: string; count: string }[] = [
  { key: "planning", label: "Planning", dot: "bg-ink-4", count: "text-ink-4" },
  { key: "implementing", label: "Implementing", dot: "bg-accent", count: "text-accent" },
  { key: "waiting", label: "Waiting", dot: "bg-warn", count: "text-warn-ink" },
  { key: "review", label: "Review", dot: "bg-info", count: "text-info" },
  { key: "merged", label: "Merged", dot: "bg-ok", count: "text-ok" },
];

export const MERGED_VISIBLE = 2;

// A live question or permission outranks the stage; an open PR outranks the stage too.
export function pipelineColumnOf(view: WorkstreamView, waiting: boolean): Exclude<PipelineKey, "merged"> {
  if (waiting) return "waiting";
  if (view.item.prNumber != null) return "review";
  switch (view.item.stage) {
    case "implement":
      return "implementing";
    case "validate":
    case "commit":
    case "ship":
    case "revise":
      return "review";
    default:
      return "planning";
  }
}

export function mergedToday(archived: ArchivedWorkstreamView[], now: number): ArchivedWorkstreamView[] {
  const start = new Date(now);
  start.setHours(0, 0, 0, 0);
  return archived
    .filter((v) => v.info.pr.state === "merged" && (v.item.archivedAt ?? 0) >= start.getTime())
    .sort((a, b) => (b.item.archivedAt ?? 0) - (a.item.archivedAt ?? 0));
}
