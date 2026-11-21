import { useRef, useState, type ReactNode } from "react";

import { openExternalUrl } from "@/lib/browser";
import type { MergeMethod, WorkstreamView } from "@/lib/bindings";
import { useBootstrap, useMergePr, useWorkstreamPr } from "@/lib/queries";
import { addressPrFeedback } from "@/features/session/sessionStore";
import { MenuItem, Popover, Spinner } from "@/components/ui";
import { Icon } from "@/components/Icon";

const pill =
  "inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md border px-2.5 text-body font-medium disabled:cursor-not-allowed disabled:opacity-50";

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
  const bootstrap = useBootstrap();
  const automerge = bootstrap.data?.settings.git?.automerge ?? false;
  const [mergeOpen, setMergeOpen] = useState(false);
  const mergeBtnRef = useRef<HTMLButtonElement>(null);

  const data = pr.data;
  if (!view || !data) return null;

  const isMerged = data.state === "merged";
  const isOpen = data.state === "open";
  const reviewRequired = data.reviewDecision === "reviewRequired";
  const changesRequested = data.reviewDecision === "changesRequested";
  const reviewBlocks = reviewRequired || changesRequested;
  const checks = data.checks.state;
  const checksOk = checks === "passing" || checks === "none";
  const directOk = checksOk && !reviewBlocks;
  const queuesAuto =
    automerge && checks !== "failing" && (checks === "pending" || reviewRequired);
  const canMerge = isOpen && data.mergeable && (directOk || queuesAuto) && !merge.isPending;

  const mergeTitle = () => {
    if (queuesAuto) return "Queue auto-merge. GitHub merges once requirements are met";
    if (canMerge) return "Merge this pull request";
    if (checks === "pending") return "Waiting for checks to finish";
    if (checks === "failing") return "Fix the failing checks before merging";
    if (reviewRequired) return "Waiting for a required review before this can merge";
    if (changesRequested) return "A reviewer requested changes. Address them before merging";
    if (!data.mergeable) return "GitHub reports this PR is not mergeable (conflicts or branch protection)";
    return "Merge unavailable";
  };

  return (
    <div className="flex shrink-0 flex-wrap items-center gap-1.5">
      <button
        type="button"
        onClick={() => void openExternalUrl(data.url)}
        title={`${data.title}\n${data.url}`}
        className={`${pill} border-edge bg-edge/40 text-ink-dim hover:bg-edge/70 hover:text-ink`}
      >
        <Icon name="pullRequest" size="sm" />
        PR #{data.number}
      </button>

      {isOpen && checks === "failing" && (
        <PillButton
          tint="border-bad/40 bg-bad/10 text-bad hover:bg-bad/20"
          onClick={() => void addressPrFeedback(id, "ci", data.number)}
          title="Hand the failing checks to the agent to fix on this branch"
        >
          Fix CI
        </PillButton>
      )}
      {isOpen && changesRequested && (
        <PillButton
          tint="border-warn/40 bg-warn/10 text-warn-ink hover:bg-warn/20"
          onClick={() => void addressPrFeedback(id, "comments", data.number)}
          title="Hand the review feedback to the agent to address on this branch"
        >
          Address comments
        </PillButton>
      )}
      {isOpen && (
        <div>
          <button
            ref={mergeBtnRef}
            type="button"
            disabled={!canMerge}
            onClick={() => setMergeOpen((o) => !o)}
            className={`${pill} border-ship/50 bg-ship/15 text-ship hover:bg-ship/25`}
            title={mergeTitle()}
          >
            {merge.isPending ? <Spinner size="xs" /> : null}
            {queuesAuto ? "Auto-merge" : "Merge"}
          </button>
          <Popover
            open={mergeOpen}
            onClose={() => setMergeOpen(false)}
            anchorRef={mergeBtnRef}
            align="end"
            className="min-w-44"
          >
            {MERGE_OPTIONS.map((opt) => (
              <MenuItem
                key={opt.method}
                onClick={() => {
                  setMergeOpen(false);
                  merge.mutate(opt.method);
                }}
              >
                {opt.label}
              </MenuItem>
            ))}
          </Popover>
        </div>
      )}
      {isMerged && (
        <PillButton
          tint="border-revise/40 bg-revise/10 text-revise hover:bg-revise/20"
          onClick={() => onArchive(view)}
          title="Merged. Removes this workstream's git worktree"
        >
          Archive
        </PillButton>
      )}
    </div>
  );
}
