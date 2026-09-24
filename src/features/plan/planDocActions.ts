import { approvePlanAndBuild } from "@/features/session/sessionStore";
import type { ShellTab } from "@/features/session/workstreamTabs";
import { commands } from "@/lib/bindings";
import { isAppError, unwrap } from "@/lib/ipc";
import { queryClient } from "@/lib/queryClient";

import { isDraftStatus, parsePlanDoc, toggleTask } from "./planDoc";

function invalidateDoc(path: string): void {
  void queryClient.invalidateQueries({ queryKey: ["doc", path] });
  void queryClient.invalidateQueries({ queryKey: ["docs"] });
}

export async function approvePlan(workstreamId: string, planPath: string, from: ShellTab): Promise<void> {
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
  await approvePlanAndBuild(workstreamId, planPath, from);
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
