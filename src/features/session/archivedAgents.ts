import { useMemo } from "react";

import { engineLabel } from "@/features/multiplexer/engines";
import type { Session, SessionState } from "@/lib/bindings";
import { harnessOption } from "@/lib/domain/harnessing";
import { claudeModels, modelOption, type ModelOption } from "@/lib/domain/models";
import { stageLabel } from "@/lib/domain/stages";
import { useClaudeProbe, useWorkstreamSessions } from "@/lib/queries";

import { deleteServerSession, setSessionArchived } from "./sessionArchive";
import { dropSession } from "./sessionStore";
import { isTerminal } from "./transcript";
import {
  openSessionTab,
  requestComposerFocus,
  unarchiveShellState,
  updateWorkstreamTabs,
  useWorkstreamTabsStore,
  type ArchivedShell,
  type ArchivedStatus,
  type ShellTab,
} from "./workstreamTabs";

/**
 * One archived agent: the server's archived session, this device's archived
 * tab, or both. A local tab carries the view state a restore brings back.
 */
export type ArchivedRow = {
  key: string;
  title: string;
  model: string;
  status: ArchivedStatus;
  archivedAt: number;
  local?: ArchivedShell;
  session?: Session;
};

const NONE_LOCAL: ArchivedShell[] = [];
const NONE_SESSIONS: Session[] = [];

function archivedModelLabel(shell: ShellTab, catalog: readonly ModelOption[]): string {
  if (shell.ptyAgent) return engineLabel(shell.ptyAgent.engineId);
  if (shell.harness === "opencode") return shell.opencodeModel ?? "OpenCode";
  if (shell.harness === "codex") return shell.codexModel ?? "Codex";
  return modelOption(shell.model, catalog).label;
}

export function classifyArchivedStatus(state: SessionState | undefined): ArchivedStatus {
  if (!state) return "idle";
  if (state === "completed") return "done";
  if (isTerminal(state)) return "stopped";
  return "idle";
}

function sessionModelLabel(session: Session, catalog: readonly ModelOption[]): string {
  if (session.harness === "claude") return modelOption(session.model ?? undefined, catalog).label;
  return session.model ?? harnessOption(session.harness).label;
}

// A local tab still archived here shows even when the server never heard of the archive.
function archivedRows(
  local: readonly ArchivedShell[],
  sessions: readonly Session[],
  catalog: readonly ModelOption[],
): ArchivedRow[] {
  const byId = new Map(sessions.map((s) => [s.id, s]));
  const claimed = new Set<string>();
  const rows: ArchivedRow[] = local.map((entry) => {
    const session = entry.shell.sessionId ? byId.get(entry.shell.sessionId) : undefined;
    if (session) claimed.add(session.id);
    return {
      key: `tab:${entry.shell.id}`,
      title: entry.shell.titleLocked ? entry.shell.title : (session?.title ?? entry.shell.title),
      model: archivedModelLabel(entry.shell, catalog),
      status: entry.status,
      archivedAt: entry.archivedAt,
      local: entry,
      session,
    };
  });
  for (const session of sessions) {
    if (session.archivedAt == null || claimed.has(session.id)) continue;
    rows.push({
      key: `session:${session.id}`,
      title: session.title ?? session.preview ?? stageLabel(session.stage),
      model: sessionModelLabel(session, catalog),
      status: classifyArchivedStatus(session.state),
      archivedAt: session.archivedAt,
      session,
    });
  }
  return rows.sort((a, b) => b.archivedAt - a.archivedAt);
}

export function useArchivedRows(workstreamId: string): ArchivedRow[] {
  const local = useWorkstreamTabsStore((s) => s.byWs[workstreamId]?.archived) ?? NONE_LOCAL;
  const sessions = useWorkstreamSessions(workstreamId).data ?? NONE_SESSIONS;
  const probed = useClaudeProbe().data?.models;
  const catalog = useMemo(() => claudeModels(probed), [probed]);
  return useMemo(() => archivedRows(local, sessions, catalog), [local, sessions, catalog]);
}

export function restoreArchivedAgent(workstreamId: string, shellId: number): void {
  let sessionId: string | undefined;
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId];
    sessionId = current?.archived?.find((a) => a.shell.id === shellId)?.shell.sessionId;
    const next = current && unarchiveShellState(current, shellId);
    return next && next !== current ? { ...all, [workstreamId]: next } : all;
  });
  if (sessionId) void setSessionArchived(workstreamId, sessionId, false);
  requestComposerFocus(shellId);
}

export function restoreArchivedRow(workstreamId: string, row: ArchivedRow): void {
  if (row.local) {
    restoreArchivedAgent(workstreamId, row.local.shell.id);
  } else if (row.session) {
    void setSessionArchived(workstreamId, row.session.id, false);
    openSessionTab(workstreamId, row.session);
  }
}

function deleteArchivedAgent(workstreamId: string, shellId: number): void {
  let sessionId: string | undefined;
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId];
    const archived = current?.archived ?? [];
    const entry = archived.find((a) => a.shell.id === shellId);
    if (!current || !entry) return all;
    sessionId = entry.shell.sessionId;
    return { ...all, [workstreamId]: { ...current, archived: archived.filter((a) => a !== entry) } };
  });
  if (sessionId) {
    dropSession(sessionId);
    void deleteServerSession(workstreamId, sessionId);
  }
}

export function deleteArchivedRow(workstreamId: string, row: ArchivedRow): void {
  if (row.local) deleteArchivedAgent(workstreamId, row.local.shell.id);
  else if (row.session) void deleteServerSession(workstreamId, row.session.id);
}

/** Drops archived tabs whose session another device deleted; `sessions` is the full list. */
export function pruneDeletedArchived(workstreamId: string, sessions: readonly Session[]): void {
  const alive = new Set(sessions.map((s) => s.id));
  const gone: string[] = [];
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId];
    const archived = current?.archived ?? [];
    const kept = archived.filter((a) => {
      const id = a.shell.sessionId;
      if (!id || alive.has(id)) return true;
      gone.push(id);
      return false;
    });
    if (!current || kept.length === archived.length) return all;
    return { ...all, [workstreamId]: { ...current, archived: kept } };
  });
  for (const id of gone) dropSession(id);
}
