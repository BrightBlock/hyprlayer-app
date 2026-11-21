import { useState } from "react";

import { openExternalUrl } from "@/lib/browser";
import type { ChecksState, ReviewDecision, WorkstreamView } from "@/lib/bindings";
import { useWorkstreamPr } from "@/lib/queries";
import { Spinner } from "@/components/ui";
import { Icon } from "@/components/Icon";

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

function CheckGlyph({ state }: { state: ChecksState }) {
  if (state === "pending") return <Spinner size="xs" />;
  if (state === "failing") return <Icon name="close" size="xs" className="text-bad-ink" />;
  return <Icon name="check" size="xs" className="text-ok" />;
}

function MergeConflictRow({ url }: { url: string }) {
  return (
    <div className="border-b border-edge px-3 py-2">
      <button
        type="button"
        onClick={() => void openExternalUrl(url)}
        title="Open the PR on GitHub to resolve the conflict"
        className="group flex w-full items-center gap-2 text-left text-ui"
      >
        <span className="w-3 shrink-0 text-center font-mono text-bad">!</span>
        <span className="min-w-0 flex-1 truncate font-medium text-bad">Merge conflict</span>
        <Icon name="external" size="xs" className="opacity-0 group-hover:opacity-60" />
      </button>
      <p className="mt-1 pl-5 text-ui-sm leading-prose text-ink-dim">
        This branch conflicts with its base branch and must be resolved before it
        can be merged. Open the PR on GitHub to resolve the conflicts.
      </p>
    </div>
  );
}

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
        onClick={() => void openExternalUrl(url)}
        title="Open the PR on GitHub"
        className="group flex w-full items-center gap-2 text-left text-ui"
      >
        <span className="w-3 shrink-0 text-center font-mono text-warn-ink">!</span>
        <span className="min-w-0 flex-1 truncate font-medium text-warn-ink">
          {changes ? "Changes requested" : "Review required"}
        </span>
        <Icon name="external" size="xs" className="opacity-0 group-hover:opacity-60" />
      </button>
      <p className="mt-1 pl-5 text-ui-sm leading-prose text-ink-dim">
        {changes
          ? "A reviewer requested changes. Address the feedback and get an approval before this can be merged."
          : "At least one approving review is required before this can be merged. The Merge button unlocks once the PR is approved."}
      </p>
    </div>
  );
}

export function PrChecksPanel({ view }: { view: WorkstreamView | null }) {
  const id = view?.item.id ?? "";
  const pr = useWorkstreamPr(id, view !== null);
  const [collapsed, setCollapsed] = useState(false);

  const data = pr.data;
  if (!view || !data || data.state !== "open") return null;

  const isConflicting = data.mergeableState === "conflicting";
  const reviewBlock =
    data.reviewDecision === "reviewRequired" || data.reviewDecision === "changesRequested"
      ? data.reviewDecision
      : null;
  const hasChecks = data.checksList.length > 0;
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
          {hasContent ? <Icon name={open ? "chevronDown" : "chevronRight"} size="xs" /> : null}
        </span>
        <StatusDot state={headerState} />
        <span className="text-label text-ink-dim">CI Checks</span>
        <span className="ml-auto text-ui text-ink-dim">
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
              onClick={() => check.url && void openExternalUrl(check.url)}
              title={check.url ?? check.name}
              className="group flex w-full items-center gap-2 px-3 py-1.5 text-left text-ui hover:bg-edge/50 disabled:cursor-default disabled:hover:bg-transparent"
            >
              <span className="w-3 shrink-0 text-center font-mono">
                <CheckGlyph state={check.state} />
              </span>
              <span className="min-w-0 flex-1 truncate text-ink">{check.name}</span>
              {check.url && <Icon name="external" size="xs" className="opacity-0 group-hover:opacity-60" />}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
