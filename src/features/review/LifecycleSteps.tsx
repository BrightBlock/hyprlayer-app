import type { ReactNode } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

import { Icon } from "@/components/Icon";
import { Button, IconButton } from "@/components/ui";
import { liveGoalForPlan, workstreamGoalLabel } from "@/lib/domain/goals";
import { plural } from "@/lib/format";
import { useGoals, useReview, useWorkstreamPr } from "@/lib/queries";
import type { WorkstreamView } from "@/lib/bindings";
import type { LifecycleStepsProps } from "@/features/plan/lifecycleSlot";
import { phaseLine } from "@/features/plan/planTracker";
import { usePlanTracker } from "@/features/plan/usePlanTracker";
import { useSession } from "@/features/session/sessionStore";
import { requestTranscriptJump } from "@/features/session/transcriptJump";
import { openSessionTab } from "@/features/session/workstreamTabs";

import { FixUntilMetButton } from "./goalActions";
import { OpenValidateButton, useRestoreImplement, ValidateActions } from "./ReadyForReviewCard";
import { reviewStatusLabel, ReviewStepBody, summarize } from "./ReviewChecklist";
import { useValidateFlow, type ValidateFlow } from "./validateFlow";
import { useImplementSessionFor } from "./validateStep";

const SHOWN_CHECKS = 4;

const OUTCOME_ICON = {
  pass: <Icon name="check" size="xs" className="text-ok" />,
  fail: <Icon name="close" size="xs" className="text-bad-ink" />,
} as const;

function ValidationSummary({ view, flow }: { view: WorkstreamView; flow: ValidateFlow }) {
  const workstreamId = view.item.id;
  const report = flow.report;
  const session = flow.validateSession ?? flow.implement;
  if (!report) {
    return flow.validateSession ? (
      <div className="flex justify-end">
        <OpenValidateButton workstreamId={workstreamId} flow={flow} />
      </div>
    ) : null;
  }
  const hidden = report.checks.length - SHOWN_CHECKS;
  const blocking = flow.phase === "issues";
  return (
    <>
      {report.gate && <p className="text-meta text-ink-dim">{report.gate}</p>}
      {report.checks.length > 0 && (
        <ul className="space-y-1">
          {report.checks.slice(0, SHOWN_CHECKS).map((c, i) => (
            <ReportRow
              key={i}
              text={c.text}
              icon={
                c.outcome ? OUTCOME_ICON[c.outcome] : <span aria-hidden="true" className="h-1 w-1 rounded-full bg-ink-4" />
              }
              onFix={
                blocking && c.outcome === "fail" && !report.judged
                  ? () => void flow.fix(`Failed check: ${c.text}`)
                  : undefined
              }
              fixing={flow.busy === "fix"}
            />
          ))}
          {hidden > 0 && <li className="pl-[18px] text-meta text-ink-dim">{plural(hidden, "more check")}</li>}
        </ul>
      )}
      {report.issues.length > 0 && (
        <div className="space-y-1">
          <div className="text-label text-ink-dim">Issues · {report.issues.length}</div>
          <ul className="space-y-1">
            {report.issues.map((text, i) => (
              <ReportRow
                key={i}
                text={text}
                icon={<Icon name="warning" size="xs" className="text-warn-ink" />}
                onFix={blocking ? () => void flow.fix(text) : undefined}
                fixing={flow.busy === "fix"}
              />
            ))}
          </ul>
        </div>
      )}
      {report.notes.length > 0 && (
        <div className="space-y-1">
          <div className="text-label text-ink-dim">Notes · {report.notes.length}</div>
          <ul className="space-y-1">
            {report.notes.map((text, i) => (
              <ReportRow key={i} text={text} icon={<span aria-hidden="true" className="h-1 w-1 rounded-full bg-ink-4" />} />
            ))}
          </ul>
        </div>
      )}
      {report.manual.length > 0 && (
        <div className="space-y-1">
          <div className="text-label text-ink-dim">Manual checks · {report.manual.length}</div>
          <ul className="space-y-1">
            {report.manual.map((text, i) => (
              <ReportRow key={i} text={text} icon={<span aria-hidden="true" className="h-1 w-1 rounded-full bg-ink-4" />} />
            ))}
          </ul>
        </div>
      )}
      <div className="flex flex-wrap items-center justify-end gap-2">
        <Button
          size="sm"
          onClick={() => {
            if (session) openSessionTab(workstreamId, session);
            requestTranscriptJump(flow.reportSessionId, { itemId: report.itemId });
          }}
        >
          Full report
        </Button>
        {blocking && (
          <>
            <Button size="sm" disabled={flow.busy !== null} onClick={flow.accept}>
              Accept
            </Button>
            <FixUntilMetButton flow={flow} view={view} />
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
          </>
        )}
      </div>
    </>
  );
}

function ReportRow({
  text,
  icon,
  onFix,
  fixing,
}: {
  text: string;
  icon: ReactNode;
  onFix?: () => void;
  fixing?: boolean;
}) {
  return (
    <li className="group flex gap-1.5 text-ui">
      <span className="flex h-4 w-3 shrink-0 items-center justify-center">{icon}</span>
      <span className="line-clamp-2 min-w-0 flex-1 text-ink-2">{text}</span>
      {onFix && (
        <IconButton
          title="Fix with agent"
          aria-label="Fix with agent"
          disabled={fixing}
          onClick={onFix}
          className="h-5 w-5 shrink-0"
        >
          <Icon name="agents" size="xs" />
        </IconButton>
      )}
    </li>
  );
}

export function LifecycleSteps({ view, sessionId, header, phases }: LifecycleStepsProps) {
  const validates = useSession(sessionId)?.stage === "validate";
  const linked = useImplementSessionFor(view.item.id, sessionId);
  const implementId = validates ? linked : sessionId;
  if (!implementId) {
    return (
      <>
        {header(IMPLEMENT_BADGE)}
        {phases}
      </>
    );
  }
  return <Stages view={view} implementId={implementId} restore={validates} header={header} phases={phases} />;
}

type Stage = "implement" | "validate" | "review" | "ship" | "shipped";

const IMPLEMENT_BADGE = { label: "Implement", className: "bg-info", detail: null };

// What validation concluded, once the plan has moved on; the report stays one click away.
function ValidationLine({ workstreamId, flow }: { workstreamId: string; flow: ValidateFlow }) {
  const label =
    flow.phase === "skipped"
      ? "Validation skipped"
      : flow.phase !== "done"
        ? null
        : flow.accepted
          ? "Validation accepted with findings"
          : flow.passed
            ? "Validation passed"
            : "Validation finished, plan not promoted";
  if (!label) return null;
  const report = flow.report;
  const session = flow.validateSession ?? flow.implement;
  return (
    <ResultRow
      done={flow.phase === "done"}
      label={label}
      action={
        report && (
          <Button
            size="sm"
            onClick={() => {
              if (session) openSessionTab(workstreamId, session);
              requestTranscriptJump(flow.reportSessionId, { itemId: report.itemId });
            }}
          >
            Report
          </Button>
        )
      }
    />
  );
}

// Fixed height, so rows with and without a button line up.
function ResultRow({ done, label, action }: { done: boolean; label: string; action?: ReactNode }) {
  return (
    <div className="flex h-6 items-center gap-1.5 text-ui">
      <Icon name={done ? "check" : "validate"} size="xs" className={done ? "shrink-0 text-ok" : "shrink-0 text-ink-4"} />
      <span className="min-w-0 flex-1 truncate text-ink-2">{label}</span>
      {action && <span className="shrink-0 [&>button]:min-w-[72px] [&>button]:justify-center">{action}</span>}
    </div>
  );
}

function validateDetail(flow: ValidateFlow): string | null {
  switch (flow.phase) {
    case "validating":
      return flow.step?.kind === "agent" ? "Running in a new agent" : "Running in this chat";
    case "summarizing":
      return "Summarizing";
    case "issues":
      return plural(flow.findings.length, "issue");
    case "fixing":
      return "Fixing";
    default:
      return null;
  }
}

function Stages({
  view,
  implementId,
  restore,
  header,
  phases,
}: Omit<LifecycleStepsProps, "sessionId"> & { implementId: string; restore: boolean }) {
  const wsId = view.item.id;
  useRestoreImplement(wsId, implementId, restore);
  const plan = usePlanTracker(implementId);
  const flow = useValidateFlow(view, implementId, plan);
  const review = useReview(wsId, plan.tracker?.tasksDone ?? false).data;
  const pr = useWorkstreamPr(wsId).data;
  const goals = useGoals(wsId).data;
  const goal = plan.key ? liveGoalForPlan(goals, plan.key) : null;

  const s = review && review.files.files.length > 0 ? summarize(review) : null;
  const approved = review?.status === "approved";
  const livePr = pr && pr.state !== "closed" ? pr : null;
  const stage: Stage =
    flow.phase === "waiting"
      ? "implement"
      : ["choosing", "validating", "summarizing", "issues", "fixing"].includes(flow.phase)
        ? "validate"
        : livePr?.state === "merged"
          ? "shipped"
          : livePr || approved
            ? "ship"
            : "review";

  const tracker = plan.tracker;
  const badge = {
    implement: { ...IMPLEMENT_BADGE, detail: tracker ? phaseLine(tracker) : null },
    validate: { label: "Validate", className: "bg-validate", detail: validateDetail(flow) },
    review: { label: "Review", className: "bg-accent", detail: review && s ? reviewStatusLabel(review, s) : null },
    ship: { label: "Ship", className: "bg-commit", detail: livePr ? `PR #${livePr.number} open` : null },
    shipped: { label: "Shipped", className: "bg-ok", detail: livePr ? `PR #${livePr.number} merged` : null },
  }[stage];
  const shown = goal
    ? { ...badge, detail: workstreamGoalLabel({ iteration: goal.iteration, maxIterations: goal.limits.maxIterations }) }
    : badge;

  const body =
    stage === "validate" ? (
      <>
        {flow.phase === "choosing" && <ValidateActions flow={flow} view={view} />}
        {flow.phase === "validating" && flow.step?.kind === "agent" && (
          <div className="flex justify-end">
            <OpenValidateButton workstreamId={wsId} flow={flow} />
          </div>
        )}
        {flow.phase === "issues" && <ValidationSummary view={view} flow={flow} />}
      </>
    ) : stage === "review" ? (
      <>
        <ValidationLine workstreamId={wsId} flow={flow} />
        {review && s ? (
          <ReviewStepBody view={view} review={review} s={s} />
        ) : (
          <p className="text-meta text-ink-dim">No changes to review</p>
        )}
      </>
    ) : stage === "ship" || stage === "shipped" ? (
      <>
        <ValidationLine workstreamId={wsId} flow={flow} />
        {livePr ? (
          <>
            {approved && <ResultRow done label="Review approved" />}
            <ResultRow
              done={livePr.state === "merged"}
              label={`PR #${livePr.number} ${livePr.state}`}
              action={
                <Button size="sm" onClick={() => void openUrl(livePr.url)}>
                  Open PR
                  <Icon name="external" size="xs" />
                </Button>
              }
            />
          </>
        ) : (
          <ResultRow
            done={approved}
            label={approved ? "Review approved" : "Not reviewed"}
            action={
              <Button size="sm" disabled={!flow.git.canShip} busy={flow.git.shipping} onClick={flow.git.doShip}>
                <Icon name="ship" size="sm" />
                Ship
              </Button>
            }
          />
        )}
        {!livePr && flow.git.shipBlocked && <p className="text-meta text-ink-dim">{flow.git.shipBlocked}</p>}
      </>
    ) : null;

  return (
    <>
      {header(shown)}
      {body && <div className="space-y-2.5 border-b border-edge px-[18px] py-4">{body}</div>}
      {phases}
    </>
  );
}
