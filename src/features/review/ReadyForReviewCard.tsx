import { useEffect } from "react";

import { Icon } from "@/components/Icon";
import { Button, ChordHint } from "@/components/ui";
import { openReview } from "@/lib/appNav";
import type { ReviewView, WorkstreamView } from "@/lib/bindings";
import { permissionFlag } from "@/lib/domain/permissions";
import { plural } from "@/lib/format";
import { effectiveChord } from "@/lib/keybindings/resolve";
import { findWorkstreamForNotify } from "@/lib/notifications";
import { IS_MACOS } from "@/lib/platform";
import { useBootstrap, useReview, useWorkstreams } from "@/lib/queries";
import type { PlanTracker } from "@/features/plan/planTracker";
import { usePlanTracker } from "@/features/plan/usePlanTracker";
import type { ReadyCardProps } from "@/features/session/readyCardSlot";
import { useSessionGoalRow } from "@/features/session/sessionGoal";
import { restoreSessionIfNeeded, useSession } from "@/features/session/sessionStore";
import { openSessionTab, sessionPermissionMode } from "@/features/session/workstreamTabs";
import type { useWorkstreamGitActions } from "@/features/workstreams/useGitActions";

import { FixUntilMetButton, ImplementUntilValidatedButton } from "./goalActions";
import { goalOwnsNextStep } from "./GoalHandoff";
import { useValidateFlow, type ValidateFlow } from "./validateFlow";
import { useImplementSessionFor } from "./validateStep";

const CARD_FRAME = "mt-4 max-w-[min(520px,100%)] rounded-card border border-edge bg-panel px-3 py-2.5";
const CARD = `${CARD_FRAME} space-y-2.5`;

export function ReadyForReviewCard({ sessionId, workstreamId }: ReadyCardProps) {
  const projectPath = findWorkstreamForNotify(workstreamId)?.projectPath;
  return projectPath ? (
    <ReadyCard sessionId={sessionId} workstreamId={workstreamId} projectPath={projectPath} />
  ) : null;
}

function ReadyCard({ sessionId, workstreamId, projectPath }: ReadyCardProps & { projectPath: string }) {
  const view = useWorkstreams(projectPath).data?.find((w) => w.item.id === workstreamId);
  const validates = useSession(sessionId)?.stage === "validate";
  const implementId = useImplementSessionFor(workstreamId, sessionId);
  if (!view) return null;
  if (!validates) return <AfterImplement implementId={sessionId} view={view} inValidateTab={false} />;
  return implementId ? <AfterImplement implementId={implementId} view={view} inValidateTab /> : null;
}

// A validate tab shows the implement session's plan, which may not be loaded yet.
export function useRestoreImplement(workstreamId: string, implementId: string, enabled: boolean): void {
  useEffect(() => {
    if (!enabled) return;
    const mode = permissionFlag(sessionPermissionMode(workstreamId, implementId));
    void restoreSessionIfNeeded(implementId, workstreamId, "implement", mode);
  }, [enabled, implementId, workstreamId]);
}

function AfterImplement({
  implementId,
  view,
  inValidateTab,
}: {
  implementId: string;
  view: WorkstreamView;
  inValidateTab: boolean;
}) {
  const workstreamId = view.item.id;
  useRestoreImplement(workstreamId, implementId, inValidateTab);
  const plan = usePlanTracker(implementId);
  const flow = useValidateFlow(view, implementId, plan);
  // get_review creates the review row, writes the exclude file and ingests notes.
  const review = useReview(workstreamId, flow.built).data;
  const tracker = plan.tracker;
  const goal = useSessionGoalRow(implementId, workstreamId);

  // Past validation, the card stays while the agent works and refreshes when it finishes.
  const reviewing = flow.phase === "done" || flow.phase === "skipped";
  if (!tracker || !(flow.built || reviewing) || !review?.files.files.length || review.status === "approved") {
    return null;
  }
  // The met goal's card offers this step itself.
  if (goalOwnsNextStep(goal, flow.step ?? null)) return null;
  if (flow.phase === "choosing") {
    return (
      <div className={CARD}>
        <div className="flex items-center gap-1.5">
          <Icon name="validate" size="sm" className="shrink-0 text-validate" />
          <span className="text-ui font-semibold text-ink">Validate the plan</span>
        </div>
        <div className="truncate text-body text-ink">{tracker.title}</div>
        <ValidateActions flow={flow} view={view} />
      </div>
    );
  }
  if (flow.phase === "summarizing" || flow.phase === "fixing") return null;
  if (flow.phase === "issues") {
    return (
      <div className={CARD}>
        <div className="flex items-center gap-1.5">
          <Icon name="warning" size="sm" className="shrink-0 text-warn-ink" />
          <span className="text-ui font-semibold text-ink">Validation found {plural(flow.findings.length, "issue")}</span>
        </div>
        <ul className="space-y-1">
          {flow.findings.slice(0, 3).map((f, i) => (
            <li key={i} className="line-clamp-2 text-ui text-ink-2">
              {f}
            </li>
          ))}
          {flow.findings.length > 3 && (
            <li className="text-meta text-ink-dim">{plural(flow.findings.length - 3, "more issue")}</li>
          )}
        </ul>
        <div className="flex items-center gap-2">
          <Button
            size="sm"
            variant="primary"
            disabled={!flow.implement || flow.busy !== null}
            busy={flow.busy === "fix"}
            onClick={() => void flow.fix()}
          >
            <Icon name="agents" size="sm" />
            Fix all
          </Button>
          <FixUntilMetButton flow={flow} view={view} />
          <Button size="sm" disabled={flow.busy !== null} onClick={flow.accept}>
            Accept
          </Button>
        </div>
      </div>
    );
  }
  if (flow.phase === "validating") {
    if (inValidateTab || flow.step?.kind !== "agent") return null;
    return (
      <div className={`${CARD_FRAME} flex items-center gap-1.5`}>
        <Icon name="validate" size="sm" className="shrink-0 text-validate" />
        <span className="flex-1 text-ui text-ink">Validating in a new agent</span>
        <OpenValidateButton workstreamId={workstreamId} flow={flow} />
      </div>
    );
  }
  return <ReviewCard tracker={tracker} view={view} review={review} git={flow.git} />;
}

export function OpenValidateButton({ workstreamId, flow }: { workstreamId: string; flow: ValidateFlow }) {
  const session = flow.validateSession;
  return session ? (
    <Button size="sm" onClick={() => openSessionTab(workstreamId, session)}>
      Open
    </Button>
  ) : null;
}

export function ValidateActions({ flow, view }: { flow: ValidateFlow; view: WorkstreamView }) {
  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          variant="primary"
          disabled={!flow.canStart}
          busy={flow.busy === "agent"}
          onClick={() => void flow.run("agent")}
        >
          Validate in new agent
        </Button>
        <Button size="sm" disabled={!flow.canStart} busy={flow.busy === "inline"} onClick={() => void flow.run("inline")}>
          Validate as subagent
        </Button>
        <span className="flex-1" />
        <Button size="sm" disabled={flow.busy !== null} onClick={flow.skip}>
          Skip
        </Button>
      </div>
      <ImplementUntilValidatedButton flow={flow} view={view} />
    </div>
  );
}

function ReviewCard({
  tracker,
  view,
  review,
  git,
}: {
  tracker: PlanTracker;
  view: WorkstreamView;
  review: ReviewView;
  git: ReturnType<typeof useWorkstreamGitActions>;
}) {
  const overrides = useBootstrap().data?.settings.keybindings?.overrides ?? {};
  const reviewChord = effectiveChord(overrides, "ws.review", IS_MACOS);
  const shipChord = effectiveChord(overrides, "ws.ship", IS_MACOS);

  const files = review.files;
  const openNotes = review.notes.filter((n) => !n.resolved).length;

  return (
    <div className={CARD}>
      <div className="flex items-center gap-1.5">
        <Icon name="review" size="sm" className="shrink-0 text-accent" />
        <span className="text-ui font-semibold text-ink">Ready for review</span>
        <span className="flex-1" />
        <span className="shrink-0 text-meta tabular-nums">
          <span className="text-diff-add">+{files.additions}</span>{" "}
          <span className="text-diff-del">−{files.deletions}</span>
        </span>
      </div>
      <div className="min-w-0 space-y-0.5">
        <div className="truncate text-body text-ink">{tracker.title}</div>
        <div className="truncate font-mono text-meta text-ink-dim">
          {view.item.branch} → {files.base}
        </div>
      </div>
      <dl className="flex gap-5 text-ui">
        <div className="flex items-baseline gap-1.5">
          <dt className="text-ink-dim">Files</dt>
          <dd className="tabular-nums text-ink">{files.files.length} changed</dd>
        </div>
        <div className="flex items-baseline gap-1.5">
          <dt className="text-ink-dim">Notes</dt>
          <dd className="tabular-nums text-ink">
            {review.notes.length} · {openNotes} unresolved
          </dd>
        </div>
      </dl>
      <div className="flex items-center gap-2">
        <Button
          size="sm"
          variant="primary"
          onClick={() => openReview({ projectPath: view.item.projectPath, workstreamId: view.item.id })}
        >
          Review changes
          {reviewChord && <ChordHint chord={reviewChord} className="text-meta opacity-70" />}
        </Button>
        <span title={git.shipBlocked ?? undefined}>
          <Button size="sm" disabled={!git.canShip} busy={git.shipping} onClick={git.doShip}>
            <Icon name="ship" size="sm" />
            Ship
            {shipChord && <ChordHint chord={shipChord} className="text-meta opacity-70" />}
          </Button>
        </span>
      </div>
    </div>
  );
}
