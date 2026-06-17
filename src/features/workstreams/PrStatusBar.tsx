import { useState, type ReactNode } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

import type { MergeMethod, WorkstreamView } from "../../lib/bindings";
import { useMergePr, useWorkstreamPr } from "../../lib/queries";
import { addressPrFeedback } from "../session/sessionStore";
import { ContextMenuItem, Spinner } from "../../components/ui";
import { GitPullRequestIcon } from "../../components/icons";

// Pill matching the header "Ship" action's height/treatment (border + tint), so
// the PR controls sit cleanly alongside it on the top row.
const pill =
  "inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md border px-2.5 text-sm font-medium disabled:cursor-not-allowed disabled:opacity-50";

/** A tinted header action pill (Fix CI / Address comments / Archive). */
function PillButton({
  tint,
  onClick,
  title,
  children,
}: {
  tint: string;
  onClick: () => void;
  title: string;
  children: ReactNode;
}) {
  return (
    <button type="button" onClick={onClick} title={title} className={`${pill} ${tint}`}>
      {children}
    </button>
  );
}

const MERGE_OPTIONS: { method: MergeMethod; label: string }[] = [
  { method: "squash", label: "Squash and merge" },
  { method: "merge", label: "Create a merge commit" },
  { method: "rebase", label: "Rebase and merge" },
];

/**
 * Pull-request controls for the header's right action cluster (next to Ship): a
 * PR link that opens in the default browser, agent feedback actions (Fix CI /
 * Address comments), a gated Merge menu, and — once merged — a purple Archive
 * button that hands off to Root's shared archive flow (`onArchive`). Renders
 * nothing until the branch has a PR, so the header is unchanged before shipping.
 */
export function PrStatusBar({
  view,
  onArchive,
}: {
  view: WorkstreamView | null;
  onArchive: (view: WorkstreamView) => void;
}) {
  const id = view?.item.id ?? "";
  const pr = useWorkstreamPr(id, view !== null);
  const merge = useMergePr(id);
  const [mergeOpen, setMergeOpen] = useState(false);

  const data = pr.data;
  if (!view || !data) return null;

  const isMerged = data.state === "merged";
  const isOpen = data.state === "open";
  // Gate on green CI + mergeable. A repo with no checks at all (`none`) has
  // nothing to wait on, so it's allowed; `pending`/`failing` are not.
  const checksOk = data.checks === "passing" || data.checks === "none";
  const canMerge = isOpen && data.mergeable && checksOk && !merge.isPending;

  const mergeTitle = () => {
    if (canMerge) return "Merge this pull request";
    if (data.checks === "pending") return "Waiting for checks to finish";
    if (data.checks === "failing") return "Fix the failing checks before merging";
    if (!data.mergeable) return "GitHub reports this PR is not mergeable (conflicts or branch protection)";
    return "Merge unavailable";
  };

  return (
    <div className="flex shrink-0 items-center gap-1.5">
      {/* The PR link, opens in the user's default browser. */}
      <button
        type="button"
        onClick={() => void openUrl(data.url)}
        title={`${data.title}\n${data.url}`}
        className={`${pill} border-edge bg-edge/40 text-ink-dim hover:bg-edge/70 hover:text-ink`}
      >
        <GitPullRequestIcon size={14} className="shrink-0" />
        PR #{data.number}
      </button>

      {isOpen && data.checks === "failing" && (
        <PillButton
          tint="border-rose-400/40 bg-rose-400/10 text-rose-300 hover:bg-rose-400/20"
          onClick={() => void addressPrFeedback(id, "ci", data.number)}
          title="Hand the failing checks to the agent to fix on this branch"
        >
          Fix CI
        </PillButton>
      )}
      {isOpen && data.reviewDecision === "CHANGES_REQUESTED" && (
        <PillButton
          tint="border-amber-400/40 bg-amber-400/10 text-amber-300 hover:bg-amber-400/20"
          onClick={() => void addressPrFeedback(id, "comments", data.number)}
          title="Hand the review feedback to the agent to address on this branch"
        >
          Address comments
        </PillButton>
      )}
      {isOpen && (
        <div className="relative">
          <button
            type="button"
            disabled={!canMerge}
            onClick={() => setMergeOpen((o) => !o)}
            className={`${pill} border-emerald-400/40 bg-emerald-400/10 text-emerald-300 hover:bg-emerald-400/20`}
            title={mergeTitle()}
          >
            {merge.isPending ? <Spinner size={12} /> : null}
            Merge
          </button>
          {mergeOpen && (
            <>
              {/* Backdrop closes the dropdown on any outside click. */}
              <div className="fixed inset-0 z-40" onClick={() => setMergeOpen(false)} />
              <div className="absolute right-0 top-full z-50 mt-1 min-w-48 rounded-md border border-edge bg-panel p-1 shadow-2xl">
                {MERGE_OPTIONS.map((opt) => (
                  <ContextMenuItem
                    key={opt.method}
                    onClick={() => {
                      setMergeOpen(false);
                      merge.mutate(opt.method);
                    }}
                  >
                    {opt.label}
                  </ContextMenuItem>
                ))}
              </div>
            </>
          )}
        </div>
      )}
      {isMerged && (
        <PillButton
          tint="border-purple-400/40 bg-purple-400/10 text-purple-300 hover:bg-purple-400/20"
          onClick={() => onArchive(view)}
          title="Merged — remove this workstream's git worktree"
        >
          Archive
        </PillButton>
      )}
    </div>
  );
}
