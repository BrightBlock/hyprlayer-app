import { approvePlanAndBuild, openGoalSession } from "@/features/session/sessionStore";
import type { ShellTab } from "@/features/session/workstreamTabs";
import {
  commands,
  type AppSettings,
  type Bootstrap,
  type ClaudeProbe,
  type Goal,
  type GoalSpec,
} from "@/lib/bindings";
import { newChatLaunchArgs } from "@/lib/domain/harnessing";
import { claudeModels, type ModelOption } from "@/lib/domain/models";
import { permissionModeOption } from "@/lib/domain/permissions";
import { isAppError, unwrap } from "@/lib/ipc";
import { queryClient } from "@/lib/queryClient";
import { invalidateGoals } from "@/lib/queries";

import { isDraftStatus, parsePlanDoc, toggleTask } from "./planDoc";

function invalidateDoc(path: string): void {
  void queryClient.invalidateQueries({ queryKey: ["doc", path] });
  void queryClient.invalidateQueries({ queryKey: ["docs"] });
}

async function activatePlan(planPath: string): Promise<void> {
  const doc = await unwrap(commands.getDoc(planPath));
  if (isDraftStatus(parsePlanDoc(doc.content, planPath).status)) {
    await unwrap(
      commands.patchDocFrontmatter(
        planPath,
        { set: [{ key: "status", value: { kind: "str", value: "active" } }] },
        doc.mtimeMs ?? 0,
      ),
    );
    invalidateDoc(planPath);
  }
}

export async function approvePlan(workstreamId: string, planPath: string, from: ShellTab): Promise<void> {
  await activatePlan(planPath);
  await approvePlanAndBuild(workstreamId, planPath, from);
}

// The agent `approvePlanAndBuild` would launch, and the permission key its tab keeps.
export function planBuildAgent(settings: AppSettings | undefined, from: ShellTab, catalog: readonly ModelOption[]) {
  const launch = newChatLaunchArgs(settings, from, catalog);
  const planMode = settings?.providers?.claude?.defaultPlanMode ? "plan" : undefined;
  return { launch, permissionKey: permissionModeOption(from.permissionMode ?? planMode).key };
}

export async function approvePlanUntilMet(
  workstreamId: string,
  planPath: string,
  from: ShellTab,
  spec: GoalSpec,
): Promise<Goal> {
  await activatePlan(planPath);
  const { launch, permissionKey } = planBuildAgent(
    queryClient.getQueryData<Bootstrap>(["bootstrap"])?.settings,
    from,
    claudeModels(queryClient.getQueryData<ClaudeProbe>(["claude-probe"])?.models),
  );
  const goal = await unwrap(
    commands.startGoal({
      target: { kind: "new_session", workstreamId, stage: "implement", ...launch, attachedDocs: [planPath] },
      spec,
    }),
  );
  invalidateGoals(goal.id);
  void queryClient.invalidateQueries({ queryKey: ["workstream-sessions"] });
  void queryClient.invalidateQueries({ queryKey: ["active-sessions"] });
  await openGoalSession(goal, {
    mode: "start",
    workstreamId,
    source: spec.source,
    planPath,
    spec,
    target: { newSession: { stage: "implement", attachedDocs: [planPath], permissionMode: permissionKey } },
  });
  return goal;
}

// The agent may be editing the same file, so a stale mtime is retried once on a fresh read.
export async function checkOffTasks(
  planPath: string,
  tasks: { key: string; title: string }[],
): Promise<void> {
  for (let attempt = 0; attempt < 2; attempt++) {
    const doc = await unwrap(commands.getDoc(planPath));
    const next = tasks.reduce((content, t) => toggleTask(content, t, true) ?? content, doc.content);
    if (next === doc.content) return;
    try {
      await unwrap(commands.saveDoc(planPath, next, doc.mtimeMs ?? 0));
      invalidateDoc(planPath);
      return;
    } catch (e) {
      if (attempt > 0 || !isAppError(e) || e.code !== "doc_conflict") throw e;
    }
  }
}
