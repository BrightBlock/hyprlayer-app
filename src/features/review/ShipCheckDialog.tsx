import { Icon } from "@/components/Icon";
import { Button, cx, Dialog, ErrorNote, Spinner } from "@/components/ui";
import { openPrompt, openReview } from "@/lib/appNav";
import type { ReviewView, WorkstreamView } from "@/lib/bindings";
import { actionAgentDefaults, harnessLabel } from "@/lib/domain/harnessing";
import { basename } from "@/lib/path";
import { useBootstrap, useReview, useWorkstreams, useWorkstreamSessions } from "@/lib/queries";
import { closeShipCheck, useShipCheck, type ShipCheckTarget } from "@/features/workstreams/shipCheck";
import { newestImplementSession, useWorkstreamGitActions } from "@/features/workstreams/useGitActions";

import { requestSubmitPopover } from "./reviewStore";

type Row = { label: string; ok: boolean; value: string; action?: { label: string; run: () => void } };

const LINK =
  "shrink-0 text-ui-sm font-semibold text-ink-2 underline decoration-edge underline-offset-2 hover:text-ink";

function readiness(review: ReviewView, target: ShipCheckTarget, agentLabel: string): Row[] {
  const { workstreamId, projectPath } = target;
  const go = (file?: string, line?: number) => {
    closeShipCheck();
    openReview({ projectPath, workstreamId, file, line });
  };
  const files = review.files.files;
  const viewed = new Set(review.viewed);
  const unviewed = files.filter((f) => !viewed.has(f.path));
  const unsent = review.comments.filter((c) => c.state === "pending" || c.state === "saved").length;
  const latest = review.rounds[review.rounds.length - 1];
  const awaitingFixes =
    review.status === "changes_requested" || (latest?.verdict === "request_changes" && !latest.delivered);
  const calls = review.notes.filter((n) => n.kind === "needs_your_call" && !n.resolved);
  const first = calls[0];

  return [
    {
      label: "Files viewed",
      ok: unviewed.length === 0,
      value:
        unviewed.length === 0
          ? `All ${files.length} viewed`
          : `${files.length - unviewed.length} of ${files.length} · ${unviewed.length} not viewed yet`,
      action: unviewed.length > 0 ? { label: "Review next", run: () => go(unviewed[0].path) } : undefined,
    },
    {
      label: "Your comments",
      ok: unsent === 0 && !awaitingFixes,
      value:
        unsent > 0
          ? `${unsent} unsent. They reach ${agentLabel} only if you request changes.`
          : awaitingFixes
            ? `Sent to ${agentLabel}. Fixes aren't reviewed yet.`
            : "None",
      action:
        unsent > 0
          ? {
              label: "Submit first",
              run: () => {
                requestSubmitPopover(workstreamId);
                go();
              },
            }
          : undefined,
    },
    {
      label: `${agentLabel}'s notes`,
      ok: !first,
      value: first
        ? `${calls.length} ${calls.length === 1 ? "needs" : "need"} your call: ${first.body.split("\n")[0]} in ${basename(first.path)}`
        : "None need your call",
      action: first ? { label: "Open", run: () => go(first.path, first.currentLine ?? undefined) } : undefined,
    },
  ];
}

export function ShipCheckDialog() {
  const target = useShipCheck((s) => s.target);
  // useWorkstreams has no `enabled`; mounting only with a target keeps it off an empty path.
  return target ? <ShipCheck target={target} /> : null;
}

function ShipCheck({ target }: { target: ShipCheckTarget }) {
  const view = useWorkstreams(target.projectPath).data?.find((w) => w.item.id === target.workstreamId);
  return view ? <ShipCheckBody target={target} view={view} /> : null;
}

function ShipCheckBody({ target, view }: { target: ShipCheckTarget; view: WorkstreamView }) {
  const git = useWorkstreamGitActions(view);
  const review = useReview(view.item.id);
  const sessions = useWorkstreamSessions(view.item.id);
  const agentLabel = harnessLabel(newestImplementSession(sessions.data ?? [])?.harness);
  const shipHarness = harnessLabel(actionAgentDefaults(useBootstrap().data?.settings).harness);
  const rows = review.data ? readiness(review.data, target, agentLabel) : null;
  const allOk = rows?.every((r) => r.ok) ?? true;
  const base = review.data?.files.base ?? view.item.baseBranch;

  const ship = () => {
    git.shipNow();
    closeShipCheck();
  };

  return (
    <Dialog
      open
      onClose={closeShipCheck}
      size="md"
      padded={false}
      ariaLabel={`Ship ${view.item.branch}?`}
      title={
        <span className="flex min-w-0 items-center gap-2 text-title">
          <Icon name="ship" size="sm" className="shrink-0" />
          <span className="min-w-0 truncate">Ship {view.item.branch}?</span>
        </span>
      }
    >
      <p className="px-4 pb-3.5 text-ui leading-prose text-ink-2">
        Runs Ship with {shipHarness}: commits, pushes the branch, and opens a pull request against{" "}
        <b className="font-semibold text-ink">{base}</b>.
      </p>
      {rows ? (
        <div className="divide-y divide-edge/60 border-t border-edge/60">
          {rows.map((r) => (
            <div key={r.label} className="flex items-center gap-2.5 px-4 py-2.5 text-ui">
              <Icon
                name={r.ok ? "check" : "warning"}
                size="sm"
                className={cx("shrink-0", r.ok ? "text-ok" : "text-warn-ink")}
              />
              <span className="w-[132px] shrink-0 truncate font-semibold text-ink">{r.label}</span>
              <span className="min-w-0 flex-1 text-ink-dim">{r.value}</span>
              {r.action && (
                <button type="button" onClick={r.action.run} className={LINK}>
                  {r.action.label}
                </button>
              )}
            </div>
          ))}
        </div>
      ) : review.isError ? (
        <div className="border-t border-edge/60 p-4">
          <ErrorNote error={review.error} />
        </div>
      ) : (
        <div className="flex justify-center border-t border-edge/60 p-4">
          <Spinner />
        </div>
      )}
      <div className="flex items-center gap-2 border-t border-edge bg-panel-2 px-4 py-3">
        <button
          type="button"
          onClick={() => {
            closeShipCheck();
            openPrompt("ship");
          }}
          className="text-ui-sm font-medium text-ink-dim underline decoration-edge underline-offset-2 hover:text-ink"
        >
          Edit the Ship prompt
        </button>
        <span className="flex-1" />
        <Button size="sm" onClick={closeShipCheck}>
          {allOk ? "Cancel" : "Keep reviewing"}
        </Button>
        <span title={git.implementWorking ? `Ship unlocks when ${agentLabel} finishes` : undefined}>
          <Button
            size="sm"
            variant="primary"
            disabled={git.implementWorking}
            busy={git.shipping}
            onClick={ship}
          >
            <Icon name="ship" size="sm" />
            {allOk ? "Ship" : "Ship anyway"}
          </Button>
        </span>
      </div>
    </Dialog>
  );
}
