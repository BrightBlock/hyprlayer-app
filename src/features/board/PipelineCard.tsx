import type { WorkstreamView } from "@/lib/bindings";
import { useSessionStore } from "@/features/session/sessionStore";
import { formatCompactAge, formatCompactCount } from "@/lib/format";
import { PIPELINE_COLUMNS, pipelineColumnOf, type PipelineWaiting } from "./pipeline";

export function PipelineCard({
  view,
  projectName,
  waiting,
  now,
  onOpen,
}: {
  view: WorkstreamView;
  projectName: string;
  waiting?: PipelineWaiting;
  now: number;
  onOpen: (view: WorkstreamView, sessionId: string | null) => void;
}) {
  const { item, git } = view;
  const column = pipelineColumnOf(view, waiting !== undefined);
  const dot = PIPELINE_COLUMNS.find((c) => c.key === column)?.dot;
  const since = useSessionStore((s) =>
    waiting?.sessionId ? (s.sessions[waiting.sessionId]?.awaitingSince ?? null) : null,
  );

  return (
    <button
      type="button"
      onClick={() => onOpen(view, waiting?.sessionId ?? null)}
      className="block w-full rounded-card border border-edge bg-panel px-3 py-[11px] text-left shadow-sm transition-colors hover:border-ink-dim/50"
      style={waiting ? { boxShadow: "inset 3px 0 0 var(--color-warn)" } : undefined}
    >
      <span className="flex items-center gap-2">
        {column !== "planning" && <span className={`h-[7px] w-[7px] shrink-0 rounded-full ${dot}`} />}
        <span className="min-w-0 flex-1 truncate text-body font-semibold text-ink">{item.title}</span>
      </span>
      <span className="mt-1 block truncate font-mono text-meta text-ink-4">{projectName}</span>
      {waiting?.line && <span className="mt-1.5 block text-ui-sm text-ink-dim">{waiting.line}</span>}
      <span className="mt-2 flex items-center gap-2">
        {git.additions > 0 && <span className="text-meta text-diff-add">+{formatCompactCount(git.additions)}</span>}
        {git.deletions > 0 && <span className="text-meta text-diff-del">−{formatCompactCount(git.deletions)}</span>}
        {item.prNumber != null && <span className="text-meta text-ink-dim">PR #{item.prNumber}</span>}
        {waiting && (
          <>
            <span className="rounded-control bg-q-chip-bg px-1.5 py-0.5 font-mono text-meta font-semibold text-q-chip-text">
              {since !== null ? `Waited ${formatCompactAge(since, now)}` : "Waiting"}
            </span>
            <span className="ml-auto text-meta font-semibold text-accent">Answer</span>
          </>
        )}
      </span>
    </button>
  );
}
