import { Icon } from "@/components/Icon";
import { Button } from "@/components/ui";
import type { WorkstreamView } from "@/lib/bindings";
import { openGoalSheet } from "@/lib/goalSheet";

import type { ValidateFlow } from "./validateFlow";

export function FixUntilMetButton({ flow, view }: { flow: ValidateFlow; view: WorkstreamView }) {
  const implement = flow.implement;
  return (
    <Button
      size="sm"
      disabled={!implement || flow.busy !== null || flow.findings.length === 0}
      onClick={() => {
        if (!implement) return;
        openGoalSheet({
          mode: "start",
          workstreamId: view.item.id,
          source: "validation",
          findings: flow.findings,
          ...(flow.planPath ? { planPath: flow.planPath } : {}),
          target: { sessionId: implement.id },
        });
      }}
    >
      <Icon name="goal" size="sm" />
      Fix until met
    </Button>
  );
}

export function ImplementUntilValidatedButton({ flow, view }: { flow: ValidateFlow; view: WorkstreamView }) {
  const { implement, planPath } = flow;
  return (
    <Button
      size="sm"
      disabled={!implement || !planPath || flow.busy !== null}
      onClick={() => {
        if (!implement || !planPath) return;
        openGoalSheet({
          mode: "start",
          workstreamId: view.item.id,
          source: "plan",
          planPath,
          withValidation: true,
          target: { sessionId: implement.id },
        });
      }}
    >
      <Icon name="goal" size="sm" />
      Implement until validated
    </Button>
  );
}
