import type { WorkstreamView } from "@/lib/bindings";
import { Spinner } from "@/components/ui";
import { isFolderWorkstream } from "@/lib/domain/projects";
import { useReprovisionWorkstream } from "@/lib/queries";

export function WorkstreamChips({ view }: { view: WorkstreamView }) {
  const reprovision = useReprovisionWorkstream();
  const { git, item } = view;

  if (isFolderWorkstream(view)) {
    return git.exists ? null : (
      <span className="shrink-0 rounded bg-warn/15 px-1.5 py-0.5 text-xs font-medium text-warn">
        folder missing on disk
      </span>
    );
  }

  const counters: string[] = [];
  if (git.dirty > 0) counters.push(`●${git.dirty}`);
  if (git.ahead > 0) counters.push(`↑${git.ahead}`);
  if (git.behind > 0) counters.push(`↓${git.behind}`);

  return (
    <div className="flex min-w-0 items-center gap-2">
      <span
        className="max-w-48 truncate rounded bg-edge/60 px-1.5 py-0.5 font-mono text-xs text-ink-dim"
        title={git.exists ? item.branch : `${item.branch} , worktree missing on disk`}
      >
        {item.branch}
      </span>
      {counters.length > 0 && (
        <span
          className="shrink-0 rounded bg-edge/60 px-1.5 py-0.5 font-mono text-xs text-ink-dim"
          title={`${git.dirty} changed · ${git.ahead} ahead · ${git.behind} behind${
            git.untracked > 0 ? ` · ${git.untracked} untracked` : ""
          }`}
        >
          {counters.join(" ")}
        </span>
      )}
      {git.thoughts === "broken" && (
        <span className="inline-flex shrink-0 items-center gap-1.5 rounded bg-warn/15 px-1.5 py-0.5 text-xs font-medium text-warn">
          thoughts broken
          <button
            type="button"
            onClick={() => reprovision.mutate(item.id)}
            disabled={reprovision.isPending}
            className="inline-flex items-center gap-1 rounded bg-warn/20 px-1 font-semibold hover:bg-warn/30 disabled:cursor-not-allowed disabled:opacity-60"
            title="Re-link thoughts for this worktree"
          >
            {reprovision.isPending && <Spinner size={10} />}
            Fix
          </button>
        </span>
      )}
    </div>
  );
}
