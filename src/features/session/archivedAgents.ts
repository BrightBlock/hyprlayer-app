import { engineLabel } from "@/features/multiplexer/engines";
import { modelOption, type ModelOption } from "@/lib/domain/models";

import { dropSession } from "./sessionStore";
import {
  requestComposerFocus,
  unarchiveShellState,
  updateWorkstreamTabs,
  type ArchivedShell,
  type ShellTab,
} from "./workstreamTabs";

export function archivedModelLabel(shell: ShellTab, catalog: readonly ModelOption[]): string {
  if (shell.ptyAgent) return engineLabel(shell.ptyAgent.engineId);
  if (shell.harness === "opencode") return shell.opencodeModel ?? "OpenCode";
  if (shell.harness === "codex") return shell.codexModel ?? "Codex";
  return modelOption(shell.model, catalog).label;
}

export const newestFirst = (archived: readonly ArchivedShell[]): ArchivedShell[] =>
  [...archived].sort((a, b) => b.archivedAt - a.archivedAt);

export function restoreArchivedAgent(workstreamId: string, shellId: number): void {
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId];
    const next = current && unarchiveShellState(current, shellId);
    return next && next !== current ? { ...all, [workstreamId]: next } : all;
  });
  requestComposerFocus(shellId);
}

export function deleteArchivedAgent(workstreamId: string, shellId: number): void {
  let sessionId: string | undefined;
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId];
    const archived = current?.archived ?? [];
    const entry = archived.find((a) => a.shell.id === shellId);
    if (!current || !entry) return all;
    sessionId = entry.shell.sessionId;
    return { ...all, [workstreamId]: { ...current, archived: archived.filter((a) => a !== entry) } };
  });
  if (sessionId) dropSession(sessionId);
}
