
import type { WorkstreamView } from "@/lib/bindings";

export function WorkstreamGitChip({ view }: { view: WorkstreamView }) {
  const { additions, deletions, dirty, ahead } = view.git;
  if (additions > 0 || deletions > 0) {
    return (
      <span className="flex shrink-0 items-center gap-1 text-meta font-medium">
        {additions > 0 && <span className="text-diff-add">+{additions}</span>}
        {deletions > 0 && <span className="text-diff-del">-{deletions}</span>}
      </span>
    );
  }
  if (dirty > 0) {
    return <span className="shrink-0 text-ui font-medium text-ink-dim">●{dirty}</span>;
  }
  if (ahead > 0) {
    return <span className="shrink-0 text-ui font-medium text-accent">↑{ahead}</span>;
  }
  return null;
}
