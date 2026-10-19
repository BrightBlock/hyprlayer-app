import { useState } from "react";

import { Icon } from "@/components/Icon";
import { Button } from "@/components/ui";
import { openReview } from "@/lib/appNav";
import type { Goal, WorkstreamView } from "@/lib/bindings";
import { errorMessage } from "@/lib/ipc";
import { findWorkstreamForNotify } from "@/lib/notifications";
import { useWorkstreams } from "@/lib/queries";
import { useToast } from "@/lib/toast";
import type { GoalHandoffProps } from "@/features/session/goalHandoffSlot";
import { validatePlanInNewSession } from "@/features/session/sessionStore";
import { useWorkstreamGitActions } from "@/features/workstreams/useGitActions";

import { useValidateStepStore } from "./validateStep";

export function GoalHandoff({ sessionId, workstreamId, goal }: GoalHandoffProps) {
  const projectPath = findWorkstreamForNotify(workstreamId)?.projectPath;
  return projectPath ? (
    <Handoff sessionId={sessionId} workstreamId={workstreamId} goal={goal} projectPath={projectPath} />
  ) : null;
}

function Handoff({ sessionId, workstreamId, goal, projectPath }: GoalHandoffProps & { projectPath: string }) {
  const view = useWorkstreams(projectPath).data?.find((w) => w.item.id === workstreamId);
  return view ? <HandoffButtons sessionId={sessionId} goal={goal} view={view} /> : null;
}

function HandoffButtons({ sessionId, goal, view }: { sessionId: string; goal: Goal; view: WorkstreamView }) {
  const git = useWorkstreamGitActions(view);
  const [validating, setValidating] = useState(false);
  // A goal that carried a Validation check already validated the plan.
  const validated = goal.checks.some((c) => c.kind === "validation");
  const planPath = (goal.source === "plan" || goal.source === "plan_phase") && !validated && goal.sourceRef;

  const validate = async (path: string) => {
    setValidating(true);
    try {
      const session = await validatePlanInNewSession(view.item.id, path, sessionId);
      // Links the validate tab to this implement session, as the validate card's own start does.
      useValidateStepStore.getState().setStep(sessionId, { kind: "agent", sessionId: session.id });
    } catch (e) {
      useToast.getState().show("Validation didn't start", errorMessage(e), { tone: "bad" });
    } finally {
      setValidating(false);
    }
  };

  return (
    <div className="mt-2.5 flex flex-wrap gap-2">
      {planPath && (
        <Button size="sm" busy={validating} disabled={validating} onClick={() => void validate(planPath)}>
          Validate plan
        </Button>
      )}
      <Button size="sm" onClick={() => openReview({ projectPath: view.item.projectPath, workstreamId: view.item.id })}>
        Review changes
      </Button>
      <Button size="sm" disabled={!git.canShip} busy={git.shipping} onClick={git.doShip}>
        <Icon name="ship" size="sm" />
        Ship
      </Button>
    </div>
  );
}
