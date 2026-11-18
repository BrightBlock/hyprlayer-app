import { useEffect, useMemo, useRef, useState } from "react";

import { Icon } from "@/components/Icon";
import { cx, ErrorNote, Spinner } from "@/components/ui";
import { useSession } from "@/features/session/sessionStore";
import { isBusy, isWorking } from "@/features/session/transcript";
import { requestTranscriptJump } from "@/features/session/transcriptJump";
import type { ShellTab } from "@/features/session/workstreamTabs";
import { repoSummary } from "@/features/statusbar/statusBarState";
import type { WorkstreamView } from "@/lib/bindings";
import {
  commandOutcomes,
  criterionOutcome,
  liveGoalForPlan,
  type CommandOutcome,
  type Goal,
} from "@/lib/domain/goals";
import { openGoalSheet, type GoalSheetTarget } from "@/lib/goalSheet";
import { useGoal, useGoals } from "@/lib/queries";
import { useUiStore } from "@/lib/uiStore";

import { BuildUntilMet } from "./BuildUntilMet";
import { HEADER_BUTTON, PlanPanel } from "./PlanPanel";
import { approvalFor, runApproval } from "./planApproval";
import { checkOffTasks } from "./planDocActions";
import { closePlanPreview } from "./planPreviewStore";
import { lifecycleStepsComponent } from "./lifecycleSlot";
import { isManualCheck, type PlanTracker, type TrackedPhase, type TrackedTask } from "./planTracker";
import { usePlanTracker } from "./usePlanTracker";

function manualCriteria(phases: TrackedPhase[]): string[] {
  return phases.flatMap((p) => p.tasks.filter(isManualCheck).map((t) => t.title));
}

// One goal list and one goal detail per plan aside, shared with the Goal view's cache keys.
function usePlanGoal(workstreamId: string, planPath: string | null) {
  const goals = useGoals(workstreamId).data;
  const goal = planPath ? liveGoalForPlan(goals, planPath) : null;
  const detail = useGoal(goal?.id ?? null).data;
  const outcomes = useMemo(() => (detail ? commandOutcomes(detail) : []), [detail]);
  return { goal, outcomes };
}

// The live implement session the aside tracks, else a new one with the plan attached.
function goalTarget(
  entryStage: string | undefined,
  live: boolean,
  sessionId: string | undefined,
  planPath: string,
): GoalSheetTarget {
  return entryStage === "implement" && live && sessionId
    ? { sessionId }
    : { newSession: { stage: "implement", attachedDocs: [planPath] } };
}

function trackingGoalProps({
  tracker,
  planPath,
  workstreamId,
  target,
  planGoal,
  outcomes,
}: {
  tracker: PlanTracker;
  planPath: string;
  workstreamId: string;
  target: GoalSheetTarget;
  planGoal: Goal | null;
  outcomes: CommandOutcome[];
}) {
  const checkStatus = (task: TrackedTask) => criterionOutcome(task, outcomes);
  if (planGoal) {
    return {
      goalAction: {
        label: `Goal ${planGoal.iteration}/${planGoal.limits.maxIterations}`,
        onClick: () => useUiStore.getState().setAsideView("goal"),
      },
      checkStatus,
    };
  }
  if (tracker.phases.length === 0) return {};
  return {
    goalAction: {
      label: "Run plan until met",
      onClick: () =>
        openGoalSheet({
          mode: "start",
          workstreamId,
          source: "plan",
          planPath,
          manual: manualCriteria(tracker.phases),
          target,
        }),
    },
    phaseAction: (phase: TrackedPhase) => (
      <button
        type="button"
        data-testid="phase-goal-action"
        data-phase={phase.label}
        onClick={() =>
          openGoalSheet({
            mode: "start",
            workstreamId,
            source: "plan_phase",
            planPath,
            phases: [phase.label],
            manual: manualCriteria([phase]),
            target,
          })
        }
        className={cx(HEADER_BUTTON, "h-6")}
      >
        <Icon name="goal" size="xs" />
        Run phase until met
      </button>
    ),
  };
}

export function PlanAside({
  view,
  shell,
  onOpenDoc,
  onOpenFile,
  onReviewChanges,
}: {
  view: WorkstreamView;
  shell: ShellTab;
  onOpenDoc: (path: string) => void;
  onOpenFile: (path: string) => void;
  onReviewChanges: () => void;
}) {
  const sessionId = shell.sessionId;
  const entry = useSession(sessionId);
  const { key, tracker, presentation, inline, indexed, loading, error, changed } = usePlanTracker(sessionId);
  const [approving, setApproving] = useState(false);
  const [approveError, setApproveError] = useState<unknown>(null);
  const planFile = key && indexed && !inline ? key : null;
  const { goal: planGoal, outcomes } = usePlanGoal(view.item.id, planFile);

  if (!key) return null;
  if (!tracker) {
    // The index can trail a fresh write by a moment; while the agent works that is not an error yet.
    const pending = loading || (entry ? isWorking(entry) : false);
    return (
      <div className="flex h-full items-center justify-center p-4">
        {pending ? <Spinner /> : <ErrorNote error={error ?? "Plan not found"} />}
      </div>
    );
  }

  const { action, blocked } = approvalFor({
    key,
    indexed,
    status: tracker.status,
    presentation,
    // Busy, not just working: a plan edit waiting on a permission prompt is still mid-write.
    busy: entry ? isBusy(entry) : false,
    pendingPermissionId: entry?.pendingPermission?.requestId ?? null,
    pendingQuestion: Boolean(entry?.pendingQuestion),
  });
  const presented = presentation?.toolUseId;
  const headerAction = inline
    ? presented && sessionId
      ? { label: "Show in chat", icon: "chat" as const, onClick: () => requestTranscriptJump(sessionId, { toolUseId: presented }) }
      : null
    : indexed
      ? { label: "Open in Library", icon: "external" as const, onClick: () => onOpenDoc(key) }
      : { label: "Open file", icon: "external" as const, onClick: () => onOpenFile(key) };
  const Steps = entry?.stage === "implement" || entry?.stage === "validate" ? lifecycleStepsComponent() : null;
  const goalProps =
    planFile && tracker.mode === "tracking"
      ? trackingGoalProps({
          tracker,
          planPath: planFile,
          workstreamId: view.item.id,
          target: goalTarget(entry?.stage, Boolean(entry?.live), sessionId, planFile),
          planGoal,
          outcomes,
        })
      : {};
  const buildGoal =
    action?.kind === "thoughts"
      ? {
          render: () => (
            <BuildUntilMet
              workstreamId={view.item.id}
              planPath={key}
              tracker={tracker}
              shell={shell}
              onError={setApproveError}
            />
          ),
        }
      : null;
  const panel = (
    <PlanPanel
      {...goalProps}
      buildGoal={buildGoal}
      lifecycle={Steps && sessionId ? (parts) => <Steps view={view} sessionId={sessionId} {...parts} /> : undefined}
      tracker={tracker}
      diff={repoSummary(view)}
      approving={approving}
      approve={action || blocked ? { blocked } : null}
      changed={changed}
      onClose={closePlanPreview}
      headerAction={headerAction}
      onReviewChanges={onReviewChanges}
      onCheckOff={planFile ? (task) => void checkOffTasks(planFile, [task]).catch(setApproveError) : undefined}
      onJump={(target) => sessionId && requestTranscriptJump(sessionId, target)}
      onApprove={() => {
        if (!action || !sessionId) return;
        setApproving(true);
        setApproveError(null);
        runApproval(action, {
          sessionId,
          workstreamId: view.item.id,
          shell,
          key,
          pendingQuestion: entry?.pendingQuestion ?? null,
        })
          .catch(setApproveError)
          .finally(() => setApproving(false));
      }}
    />
  );

  return (
    <div className="flex h-full min-h-0 flex-col">
      {approveError != null && (
        <div className="shrink-0 p-2">
          <ErrorNote error={approveError} onDismiss={() => setApproveError(null)} />
        </div>
      )}
      <div className="min-h-0 flex-1">{panel}</div>
    </div>
  );
}

// Writes `[x]` for tasks a live agent completed; tasks already complete when this mounted are left alone.
export function PlanDocSync({ sessionId }: { sessionId: string }) {
  const entry = useSession(sessionId);
  const { key: planKey, indexed, tracker } = usePlanTracker(sessionId);
  // Only a thoughts doc takes `[x]` write-backs; Claude's and inline plans are not ours to edit.
  const path = indexed ? planKey : null;
  const handled = useRef<Set<string> | null>(null);
  const live = Boolean(entry?.live);
  const writeBacks = tracker?.mode === "tracking" ? tracker.writeBacks : null;
  const key = writeBacks?.map((t) => `${t.key}\t${t.title}`).join("\n") ?? "";

  useEffect(() => {
    if (!path || !writeBacks) return;
    const ids = writeBacks.map((t) => `${t.key}\t${t.title}`);
    if (!handled.current) {
      handled.current = new Set(ids);
      return;
    }
    if (!live) return;
    const fresh = writeBacks.filter((_, i) => !handled.current!.has(ids[i]));
    if (fresh.length === 0) return;
    for (const id of ids) handled.current.add(id);
    void checkOffTasks(path, fresh).catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed on the write-back set
  }, [path, key, live]);

  return null;
}
