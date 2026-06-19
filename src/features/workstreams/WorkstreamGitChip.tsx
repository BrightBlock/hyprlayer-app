/** Conductor-style status chip for a workstream: added/deleted line counts
 *  (matching the diff panel), falling back to an ahead-of-base indicator once
 *  the working tree is clean but commits are unpushed. Used by the sidebar row
 *  and the kanban card so the numbers are visually identical in both places. */

import type { WorkstreamView } from "../../lib/bindings";

export function WorkstreamGitChip({ view }: { view: WorkstreamView }) {
  const { additions, deletions, dirty, ahead } = view.git;
  if (additions > 0 || deletions > 0) {
    return (
      <span className="flex shrink-0 items-center gap-1 font-mono text-xs font-medium">
        {additions > 0 && <span className="text-ok">+{additions}</span>}
        {deletions > 0 && <span className="text-bad">-{deletions}</span>}
      </span>
    );
  }
  // Changes with no line delta (binary edits, mode/rename only) still count.
  if (dirty > 0) {
    return <span className="shrink-0 text-xs font-medium text-ink-dim">●{dirty}</span>;
  }
  if (ahead > 0) {
    return <span className="shrink-0 text-xs font-medium text-accent">↑{ahead}</span>;
  }
  return null;
}
