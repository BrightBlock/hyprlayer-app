import { useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

import type { ChecksState, ReviewDecision, WorkstreamView } from "@/lib/bindings";
import { useWorkstreamPr } from "@/lib/queries";
import { Spinner } from "@/components/ui";
import { ChevronDownIcon, ChevronRightIcon } from "@/components/icons";

/** Overall-status dot for the CI Checks header: green passing, red failing,
 *  amber (pulsing) pending, dim when there are no checks. */
function StatusDot({ state }: { state: ChecksState }) {
  const tone =
    state === "passing"
      ? "bg-ok"
      : state === "failing"
        ? "bg-bad"
        : state === "pending"
          ? "bg-warn animate-pulse"
          : "bg-ink-dim/40";
  return <span className={`h-2 w-2 shrink-0 rounded-full ${tone}`} />;
}

/** Glyph + colour for a single check's outcome. */
function CheckGlyph({ state }: { state: ChecksState }) {
  if (state === "pending") return <Spinner size={11} />;
  if (state === "failing") return <span className="text-bad">✕</span>;
  return <span className="text-ok">✓</span>; // passing
}

function ExternalLinkIcon() {
  return (
    <svg aria-hidden="true" width="11" height="11" viewBox="0 0 24 24" fill="none" className="shrink-0 opacity-0 group-hover:opacity-60">
      <path d="M14 4h6v6M20 4l-9 9" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M19 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1h5" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

/** A leading row in the expanded list for a branch that conflicts with its
 *  base. GitHub's CI rollup never reports this, so without it a conflicting PR
 *  looks all-green here while the Merge button is silently disabled. Opens the
 *  PR on GitHub, where the conflict is resolved. */
function MergeConflictRow({ url }: { url: string }) {
  return (
    <div className="border-b border-edge px-3 py-2">
      <button
        type="button"
        onClick={() => void openUrl(url)}
        title="Open the PR on GitHub to resolve the conflict"
        className="group flex w-full items-center gap-2 text-left text-xs"
      >
        <span className="w-3 shrink-0 text-center font-mono text-bad">!</span>
        <span className="min-w-0 flex-1 truncate font-medium text-bad">Merge conflict</span>
        <ExternalLinkIcon />
      </button>
      <p className="mt-1 pl-5 text-[11px] leading-relaxed text-ink-dim">
        This branch conflicts with its base branch and must be resolved before it
        can be merged. Open the PR on GitHub to resolve the conflicts.
      </p>
    </div>
  );
}

/** A leading row for a PR whose required review isn't satisfied — waiting on a
 *  reviewer, or with changes requested. Like the merge-conflict row, GitHub's CI
 *  rollup never reports this, so without it a review-blocked PR looks all-green
 *  here while the Merge button is disabled. Warn (amber) tone — it's a wait on a
 *  person, not a failed check. Opens the PR on GitHub. */
function ReviewBlockRow({
  decision,
  url,
}: {
  decision: Exclude<ReviewDecision, "approved">;
  url: string;
}) {
  const changes = decision === "changesRequested";
  return (
    <div className="border-b border-edge px-3 py-2">
      <button
        type="button"
        onClick={() => void openUrl(url)}
        title="Open the PR on GitHub"
        className="group flex w-full items-center gap-2 text-left text-xs"
      >
        <span className="w-3 shrink-0 text-center font-mono text-warn">!</span>
        <span className="min-w-0 flex-1 truncate font-medium text-warn">
          {changes ? "Changes requested" : "Review required"}
        </span>
        <ExternalLinkIcon />
      </button>
      <p className="mt-1 pl-5 text-[11px] leading-relaxed text-ink-dim">
        {changes
          ? "A reviewer requested changes. Address the feedback and get an approval before this can be merged."
          : "At least one approving review is required before this can be merged. The Merge button unlocks once the PR is approved."}
      </p>
    </div>
  );
}

/**
 * The CI Checks breakdown at the bottom of the diff sidebar. Always shown for an
 * open PR but minimized to just its header until there's something to show —
 * actual checks, a merge conflict, or an unsatisfied required review. Once there
 * is, the section is collapsable: it leads with the synthetic blocker rows
 * GitHub's CI rollup never reports — a merge-conflict row when the branch
 * conflicts with its base, and a review row while a required review is
 * outstanding or changes were requested — then lists each check line by line
 * (glyph + name), each row opening that check inside the PR's Checks tab (Actions
 * runs) or its external CI page (third-party contexts). An overall status dot in
 * the header reflects the combined result: red on a conflict or failing checks,
 * amber while waiting on a review. Renders nothing when there's no open PR.
 */
export function PrChecksPanel({ view }: { view: WorkstreamView | null }) {
  const id = view?.item.id ?? "";
  const pr = useWorkstreamPr(id, view !== null);
  const [collapsed, setCollapsed] = useState(false);

  const data = pr.data;
  if (!view || !data || data.state !== "open") return null;

  const isConflicting = data.mergeableState === "conflicting";
  // The review decision when it blocks merging (waiting on a review, or changes
  // requested), else null — narrowed so it can drive both the row and the header.
  const reviewBlock =
    data.reviewDecision === "reviewRequired" || data.reviewDecision === "changesRequested"
      ? data.reviewDecision
      : null;
  const hasChecks = data.checksList.length > 0;
  // A conflict or an unsatisfied required review blocks merging even with no/green
  // checks, so it opens the section and colours the header dot: red for a conflict
  // or failing checks (hard problems), amber for a review still waiting on a person
  // when checks are otherwise clean.
  const hasContent = hasChecks || isConflicting || reviewBlock !== null;
  const open = hasContent && !collapsed;
  const base: ChecksState = isConflicting ? "failing" : data.checks;
  const reviewWaiting = reviewBlock !== null && (base === "passing" || base === "none");
  const headerState: ChecksState = reviewWaiting ? "pending" : base;

  return (
    <div className="flex max-h-[45%] shrink-0 flex-col border-t border-edge">
      <button
        type="button"
        disabled={!hasContent}
        onClick={() => setCollapsed((c) => !c)}
        className="flex h-9 shrink-0 items-center gap-2 px-3 text-left disabled:cursor-default"
      >
        <span className="flex w-3 shrink-0 justify-center text-ink-dim">
          {hasContent ? (
            open ? (
              <ChevronDownIcon size={12} />
            ) : (
              <ChevronRightIcon size={12} />
            )
          ) : null}
        </span>
        <StatusDot state={headerState} />
        <span className="text-xs font-semibold uppercase tracking-wide text-ink-dim">CI Checks</span>
        <span className="ml-auto text-xs text-ink-dim">
          {isConflicting
            ? "conflict"
            : reviewWaiting
              ? "review"
              : hasChecks
                ? `${data.checksPassed}/${data.checksTotal}`
                : "none yet"}
        </span>
      </button>
      {open && (
        <div className="min-h-0 flex-1 overflow-y-auto border-t border-edge py-1">
          {isConflicting && <MergeConflictRow url={data.url} />}
          {reviewBlock && <ReviewBlockRow decision={reviewBlock} url={data.url} />}
          {data.checksList.map((check, i) => (
            <button
              key={`${check.name}-${i}`}
              type="button"
              disabled={!check.url}
              onClick={() => check.url && void openUrl(check.url)}
              title={check.url ?? check.name}
              className="group flex w-full items-center gap-2 px-3 py-1.5 text-left text-xs hover:bg-edge/40 disabled:cursor-default disabled:hover:bg-transparent"
            >
              <span className="w-3 shrink-0 text-center font-mono">
                <CheckGlyph state={check.state} />
              </span>
              <span className="min-w-0 flex-1 truncate text-ink">{check.name}</span>
              {check.url && <ExternalLinkIcon />}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
