import { selectAgentTab } from "@/features/search/selectAgentTab";
import { openPromptTab, useWorkstreamTabsStore } from "@/features/session/workstreamTabs";
import { goToWorkstream } from "@/lib/appNav";
import { commands, type WorkstreamView } from "@/lib/bindings";
import { stageLabel } from "@/lib/domain/stages";
import { unwrap } from "@/lib/ipc";
import { useRestoreWorkstream } from "@/lib/queries";

/** Focuses the latest session's tab, opening one when the tab is gone. */
async function reopenLatestSession(workstreamId: string): Promise<void> {
  const sessions = await unwrap(commands.listSessions(workstreamId)).catch(() => []);
  const latest = sessions.reduce<(typeof sessions)[number] | null>(
    (best, s) => (!best || (s.startedAt ?? 0) > (best.startedAt ?? 0) ? s : best),
    null,
  );
  if (!latest) return;
  const tabs = useWorkstreamTabsStore.getState().byWs[workstreamId];
  if (tabs?.shells.some((shell) => shell.sessionId === latest.id)) {
    selectAgentTab(workstreamId, latest.id);
  } else {
    openPromptTab(workstreamId, latest.id, stageLabel(latest.stage));
  }
}

/** Restores, closes the popover, then opens the workstream on its latest session. */
export function useRestoreAndOpen(onRestored: () => void) {
  const restore = useRestoreWorkstream();
  const run = async (id: string, target: string | null = null): Promise<WorkstreamView> => {
    const view = await restore.mutateAsync({ id, target });
    onRestored();
    goToWorkstream({ projectPath: view.item.projectPath, workstreamId: view.item.id });
    await reopenLatestSession(view.item.id);
    return view;
  };
  return { run, pending: restore.isPending, error: restore.error, reset: restore.reset };
}
