import { useState, type RefObject } from "react";

import { Button, cx, ErrorNote, Popover, TextArea } from "@/components/ui";
import type { ReviewVerdict, SubmitReviewResult } from "@/lib/bindings";
import { plural } from "@/lib/format";
import { useSubmitReview } from "@/lib/queries";
import { useToast } from "@/lib/toast";
import { useRovingRadio } from "@/lib/useRovingRadio";

type Option = { value: ReviewVerdict; label: string; detail: string; disabled?: boolean };

function primaryLabel(verdict: ReviewVerdict, unsent: number, agentLabel: string): string {
  switch (verdict) {
    case "request_changes":
      return unsent > 0 ? `Send ${plural(unsent, "comment")} to ${agentLabel}` : "Request changes";
    case "approve":
      return "Approve";
    case "comment":
      return "Save comments";
  }
}

export function SubmitReviewPopover({
  open,
  onClose,
  anchorRef,
  workstreamId,
  unsent,
  viewedCount,
  total,
  agentLabel,
  agentBusy,
  onRequested,
}: {
  open: boolean;
  onClose: () => void;
  anchorRef: RefObject<HTMLElement | null>;
  workstreamId: string;
  unsent: number;
  viewedCount: number;
  total: number;
  agentLabel: string;
  agentBusy: boolean;
  onRequested: (result: SubmitReviewResult) => void;
}) {
  return (
    <Popover
      open={open}
      onClose={onClose}
      anchorRef={anchorRef}
      align="end"
      trapFocus
      aria-label="Submit review"
      className="w-[380px] p-0"
    >
      {open && (
        <SubmitForm
          workstreamId={workstreamId}
          unsent={unsent}
          viewedCount={viewedCount}
          total={total}
          agentLabel={agentLabel}
          agentBusy={agentBusy}
          onClose={onClose}
          onRequested={onRequested}
        />
      )}
    </Popover>
  );
}

function SubmitForm({
  workstreamId,
  unsent,
  viewedCount,
  total,
  agentLabel,
  agentBusy,
  onClose,
  onRequested,
}: {
  workstreamId: string;
  unsent: number;
  viewedCount: number;
  total: number;
  agentLabel: string;
  agentBusy: boolean;
  onClose: () => void;
  onRequested: (result: SubmitReviewResult) => void;
}) {
  const [verdict, setVerdict] = useState<ReviewVerdict>(() =>
    unsent === 0 ? "approve" : agentBusy ? "comment" : "request_changes",
  );
  const [summary, setSummary] = useState("");
  const submit = useSubmitReview();

  const options: Option[] = [
    { value: "comment", label: "Comment", detail: `Keep your notes on the review. ${agentLabel} won't act on them.` },
    {
      value: "request_changes",
      label: "Request changes",
      detail: agentBusy
        ? `${agentLabel} is still working`
        : `Send your comments to ${agentLabel} as a fix task in this session. Files it touches go back to unviewed.`,
      disabled: agentBusy,
    },
    {
      value: "approve",
      label: "Approve",
      detail: "Mark the review done. Ship stays one click away; nothing is pushed yet.",
    },
  ];
  const { groupProps, itemProps } = useRovingRadio(options, verdict, setVerdict);

  const needsSummary = unsent === 0 && verdict !== "approve" && !summary.trim();
  const blocked = verdict === "request_changes" && agentBusy;

  const send = () => {
    if (needsSummary || blocked || submit.isPending) return;
    submit.mutate(
      { workstreamId, verdict, summary: summary.trim() || null },
      {
        onSuccess: (result) => {
          onClose();
          const toast = useToast.getState();
          if (verdict === "request_changes") onRequested(result);
          else if (verdict === "approve") toast.show("Approved", "Nothing is pushed until you click Ship");
          else toast.show("Comments saved", "");
        },
      },
    );
  };

  return (
    <div className="space-y-3 p-3">
      {unsent > 0 && (
        <p className="text-ui text-ink-2">
          {plural(unsent, "comment")} {unsent === 1 ? "is" : "are"} waiting for a verdict.
        </p>
      )}
      <TextArea
        autoFocus
        value={summary}
        onChange={(e) => setSummary(e.target.value)}
        placeholder="Summary (optional)"
        aria-label="Summary"
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            send();
          }
        }}
      />
      <div {...groupProps} aria-label="Verdict" className="space-y-1">
        {options.map((o, i) => {
          const selected = o.value === verdict;
          return (
            <button
              key={o.value}
              type="button"
              {...itemProps(i)}
              className={cx(
                "flex w-full items-start gap-2.5 rounded-control border px-2.5 py-2 text-left",
                "focus:outline-none",
                "disabled:cursor-not-allowed disabled:opacity-50",
                selected ? "border-ink-dim/70 bg-panel-2" : "border-transparent enabled:hover:bg-edge/50 enabled:focus-visible:bg-edge/50",
              )}
            >
              <span
                aria-hidden="true"
                className={cx(
                  "mt-0.5 block h-3.5 w-3.5 shrink-0 rounded-full border-[1.5px]",
                  selected ? "border-[4px] border-ink" : "border-ink-4",
                )}
              />
              <span className="min-w-0">
                <span className="block text-ui font-medium text-ink">{o.label}</span>
                <span className="block text-meta text-ink-dim">{o.detail}</span>
              </span>
            </button>
          );
        })}
      </div>
      {submit.error != null && <ErrorNote error={submit.error} onDismiss={() => submit.reset()} />}
      <div className="flex items-center gap-2">
        <span className="flex-1 text-meta tabular-nums text-ink-dim">
          {viewedCount} / {total} viewed
        </span>
        <Button size="sm" onClick={onClose}>
          Cancel
        </Button>
        <Button
          size="sm"
          variant="primary"
          disabled={needsSummary || blocked}
          busy={submit.isPending}
          onClick={send}
        >
          {primaryLabel(verdict, unsent, agentLabel)}
        </Button>
      </div>
    </div>
  );
}
