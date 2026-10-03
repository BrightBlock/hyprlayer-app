
import { create } from "zustand";
import { persist } from "zustand/middleware";

import { scopedKey } from "@/lib/hostScope";
import type { Session } from "@/lib/bindings";

import type { Attachment } from "./attachments";
import { debouncedJSONStorage } from "@/lib/persistedStorage";
import {
  isSupportedOpencodeModel,
  type ActionAgentDefaults,
} from "@/lib/domain/harnessing";
import { MODE_STAGES, stageLabel, type StageKey } from "@/lib/domain/stages";
import { markTitleSeen } from "./titleSeen";

import {
  leaf,
  leaves,
  newPaneId,
  removeLeaf,
  splitLeaf,
  type LayoutNode,
  type PaneId,
  type SplitDir,
} from "./paneTree";

export type AttachedDoc = {
  path: string;
  role: string;
  suggested?: boolean;
};

export type ShellTab = {
  id: number;
  title: string;
  titleLocked?: boolean;
  mode: StageKey;
  /** The session's real stage when `mode` is downgraded to "general" (a picked ship/commit/revise session). */
  sessionStage?: StageKey;
  draft: string;
  docs: AttachedDoc[];
  dismissedSuggestions?: string[];
  handoffNudgeDismissed?: boolean;
  attachments: Attachment[];
  model?: string;
  effort?: string;
  permissionMode?: string;
  harness?: string;
  opencodeModel?: string;
  codexModel?: string;
  codexEffort?: string;
  sessionId?: string;
  docPreviewPath?: string;
  diffView?: { path: string; staged: boolean; sha?: string };
  filePath?: string;
  terminalView?: { terminalId?: string };
  ptyAgent?: {
    engineId: string;
    terminalId?: string;
    bypass?: boolean;
    sessionId?: string;
    started?: boolean;
  };
  paneId: PaneId;
};

export type WsTabState = {
  shells: ShellTab[];
  layout: LayoutNode;
  panes: Record<PaneId, { activeId: number }>;
  activePaneId: PaneId;
  archived?: ArchivedShell[];
};

export type ArchivedStatus = "done" | "stopped" | "idle";

export type ArchivedShell = {
  shell: ShellTab;
  archivedAt: number;
  status: ArchivedStatus;
};

export const defaultTabState = (): WsTabState => ({
  shells: [
    { id: 1, title: "Agent", mode: "general", draft: "", docs: [], attachments: [], paneId: "p1" },
  ],
  layout: leaf("p1"),
  panes: { p1: { activeId: 1 } },
  activePaneId: "p1",
});

export const nextShellId = (shells: ShellTab[], archived: ArchivedShell[] = []): number =>
  Math.max(0, ...shells.map((s) => s.id), ...archived.map((a) => a.shell.id)) + 1;

export function paneOf(state: WsTabState, shell: ShellTab): PaneId {
  const ids = leaves(state.layout);
  return ids.includes(shell.paneId) ? shell.paneId : ids[0];
}

export function focusedPaneId(state: WsTabState): PaneId {
  const ids = leaves(state.layout);
  return ids.includes(state.activePaneId) ? state.activePaneId : ids[0];
}

export const paneShells = (state: WsTabState, paneId: PaneId): ShellTab[] =>
  state.shells.filter((s) => paneOf(state, s) === paneId);

export function activeShellIn(state: WsTabState, paneId: PaneId): ShellTab | null {
  const members = paneShells(state, paneId);
  return members.find((s) => s.id === state.panes[paneId]?.activeId) ?? members[0] ?? null;
}

export const visibleShells = (state: WsTabState): ShellTab[] =>
  leaves(state.layout).flatMap((paneId) => activeShellIn(state, paneId) ?? []);

export function isLastTab(state: WsTabState, shellId: number): boolean {
  const shell = state.shells.find((s) => s.id === shellId);
  return (
    !!shell &&
    leaves(state.layout).length === 1 &&
    paneShells(state, paneOf(state, shell)).length === 1
  );
}

export function focusShell(state: WsTabState, shellId: number): WsTabState {
  const shell = state.shells.find((s) => s.id === shellId);
  if (!shell) return state;
  const paneId = paneOf(state, shell);
  return {
    ...state,
    panes: { ...state.panes, [paneId]: { activeId: shellId } },
    activePaneId: paneId,
  };
}

export function setFocusedPane(state: WsTabState, paneId: PaneId): WsTabState {
  if (state.activePaneId === paneId || !leaves(state.layout).includes(paneId)) return state;
  return { ...state, activePaneId: paneId };
}

export const appendShell = (state: WsTabState, shell: ShellTab): WsTabState =>
  focusShell({ ...state, shells: [...state.shells, shell] }, shell.id);

function dropLeaf(state: WsTabState, paneId: PaneId): WsTabState {
  const removed = removeLeaf(state.layout, paneId);
  if (!removed) return state;
  const panes = { ...state.panes };
  delete panes[paneId];
  return {
    ...state,
    layout: removed.layout,
    panes,
    activePaneId: focusedPaneId(state) === paneId ? removed.heir : state.activePaneId,
  };
}

function vacatePane(
  prev: WsTabState,
  next: WsTabState,
  shellId: number,
  paneId: PaneId,
): WsTabState {
  const before = paneShells(prev, paneId);
  const after = before.filter((s) => s.id !== shellId);
  if (after.length === 0) return dropLeaf(next, paneId);
  if (prev.panes[paneId]?.activeId !== shellId) return next;
  const successor = after[Math.min(before.findIndex((s) => s.id === shellId), after.length - 1)];
  return { ...next, panes: { ...next.panes, [paneId]: { activeId: successor.id } } };
}

export function moveShellState(state: WsTabState, shellId: number, toPaneId: PaneId): WsTabState {
  const shell = state.shells.find((s) => s.id === shellId);
  if (!shell || !leaves(state.layout).includes(toPaneId)) return state;
  const fromPaneId = paneOf(state, shell);
  if (fromPaneId === toPaneId) return state;
  const moved = {
    ...state,
    shells: state.shells.map((s) => (s.id === shellId ? { ...s, paneId: toPaneId } : s)),
  };
  return focusShell(vacatePane(state, moved, shellId, fromPaneId), shellId);
}

export function splitPaneState(
  state: WsTabState,
  targetPaneId: PaneId,
  dir: SplitDir,
  place: "before" | "after",
  shellId: number,
): WsTabState {
  const shell = state.shells.find((s) => s.id === shellId);
  if (!shell) return state;
  if (paneOf(state, shell) === targetPaneId && paneShells(state, targetPaneId).length === 1) {
    return state;
  }
  const newId = newPaneId(state.layout);
  const layout = splitLeaf(state.layout, targetPaneId, dir, place, newId);
  return layout === state.layout ? state : moveShellState({ ...state, layout }, shellId, newId);
}

export function mergePaneState(state: WsTabState, paneId: PaneId): WsTabState {
  const removed = removeLeaf(state.layout, paneId);
  if (!removed) return state;
  const inPane = (s: ShellTab) => paneOf(state, s) === paneId;
  const merged = state.shells.filter(inPane).map((s) => ({ ...s, paneId: removed.heir }));
  const shells = [...state.shells.filter((s) => !inPane(s)), ...merged];
  return dropLeaf({ ...state, shells }, paneId);
}

export function closePaneState(state: WsTabState, paneId: PaneId): WsTabState {
  const next = dropLeaf(state, paneId);
  if (next === state) return state;
  return { ...next, shells: state.shells.filter((s) => paneOf(state, s) !== paneId) };
}

export function cycleActiveTab(state: WsTabState, dir: 1 | -1): WsTabState {
  const paneId = focusedPaneId(state);
  const members = paneShells(state, paneId);
  if (members.length < 2) return state;
  const from = Math.max(0, members.findIndex((s) => s.id === state.panes[paneId]?.activeId));
  return focusShell(state, members[(from + dir + members.length) % members.length].id);
}

export function selectTabAt(state: WsTabState, index: number): WsTabState {
  const target = paneShells(state, focusedPaneId(state))[index];
  return target ? focusShell(state, target.id) : state;
}

export function selectLastTab(state: WsTabState): WsTabState {
  const members = paneShells(state, focusedPaneId(state));
  const target = members[members.length - 1];
  return target ? focusShell(state, target.id) : state;
}

export function removeShellState(state: WsTabState, shellId: number): WsTabState {
  const shell = state.shells.find((s) => s.id === shellId);
  if (!shell || isLastTab(state, shellId)) return state;
  const remaining = { ...state, shells: state.shells.filter((s) => s.id !== shellId) };
  return vacatePane(state, remaining, shellId, paneOf(state, shell));
}

export function archiveShellState(
  state: WsTabState,
  shellId: number,
  status: ArchivedStatus,
  archivedAt: number,
): WsTabState {
  const shell = state.shells.find((s) => s.id === shellId);
  if (!shell) return state;
  const next = removeShellState(state, shellId);
  if (next === state) return state;
  return { ...next, archived: [{ shell, archivedAt, status }, ...(state.archived ?? [])] };
}

export function unarchiveShellState(state: WsTabState, shellId: number): WsTabState {
  const archived = state.archived ?? [];
  const entry = archived.find((a) => a.shell.id === shellId);
  if (!entry) return state;
  return appendShell(
    { ...state, archived: archived.filter((a) => a.shell.id !== shellId) },
    { ...entry.shell, paneId: focusedPaneId(state) },
  );
}

type TabsStore = { byWs: Record<string, WsTabState> };

function normalizePersistedShell(shell: ShellTab): ShellTab {
  return {
    ...shell,
    docs: shell.docs ?? [],
    attachments: shell.attachments ?? [],
    dismissedSuggestions: shell.dismissedSuggestions ?? [],
    titleLocked: shell.titleLocked ?? true,
    opencodeModel:
      shell.opencodeModel && isSupportedOpencodeModel(shell.opencodeModel)
        ? shell.opencodeModel
        : undefined,
  };
}

type LegacyShell = ShellTab & { panel?: "main" | "right" };
type LegacyWs = Partial<WsTabState> & {
  shells: LegacyShell[];
  activeId?: number;
  rightSplit?: boolean;
  rightActiveId?: number;
  activePane?: "main" | "right";
  archiveOpen?: boolean;
};

function migrateWorkstream(ws: LegacyWs): void {
  const shells: LegacyShell[] = (ws.shells ?? [])
    .filter((shell) => !!shell)
    .map(normalizePersistedShell);
  const archived = (ws.archived ?? [])
    .filter((entry) => !!entry?.shell)
    .map((entry) => ({ ...entry, shell: normalizePersistedShell(entry.shell) }));
  ws.shells = shells;
  ws.archived = archived;

  if (!ws.layout) {
    const rightShells = shells.filter((s) => s.panel === "right");
    const split = ws.rightSplit === true && rightShells.length > 0;
    ws.layout = split
      ? { type: "split", dir: "row", ratio: 0.5, a: leaf("p1"), b: leaf("p2") }
      : leaf("p1");
    ws.panes = {
      p1: { activeId: ws.activeId ?? 0 },
      ...(split ? { p2: { activeId: ws.rightActiveId ?? rightShells[0].id } } : {}),
    };
    ws.activePaneId = split && ws.activePane === "right" ? "p2" : "p1";
    for (const s of shells) s.paneId = split && s.panel === "right" ? "p2" : "p1";
  }

  ws.panes ??= {};
  const ids = leaves(ws.layout);
  for (const s of shells) if (!ids.includes(s.paneId)) s.paneId = ids[0];
  for (const paneId of ids) {
    if (shells.some((s) => s.paneId === paneId)) continue;
    const removed = removeLeaf(ws.layout, paneId);
    if (!removed) break;
    ws.layout = removed.layout;
    delete ws.panes[paneId];
  }
  const kept = leaves(ws.layout);
  for (const paneId of kept) {
    const members = shells.filter((s) => s.paneId === paneId);
    const active = ws.panes[paneId]?.activeId;
    if (members.length > 0 && !members.some((s) => s.id === active)) {
      ws.panes[paneId] = { activeId: members[0].id };
    }
  }
  for (const a of archived) if (!kept.includes(a.shell.paneId)) a.shell.paneId = kept[0];
  if (!ws.activePaneId || !kept.includes(ws.activePaneId)) ws.activePaneId = kept[0];

  delete ws.activeId;
  delete ws.rightSplit;
  delete ws.rightActiveId;
  delete ws.activePane;
  delete ws.archiveOpen;
  for (const s of shells) delete s.panel;
  for (const a of archived) delete (a.shell as LegacyShell).panel;
}

export function migratePersistedWorkstreamTabs(persisted: unknown): TabsStore {
  const state = persisted as TabsStore | undefined;
  if (!state?.byWs) return { byWs: {} };
  for (const ws of Object.values(state.byWs)) {
    if (ws) migrateWorkstream(ws);
  }
  return state;
}

export const useWorkstreamTabsStore = create<TabsStore>()(
  persist(() => ({ byWs: {} }), {
    name: scopedKey("hyprlayer:workstream-tabs"),
    storage: debouncedJSONStorage(),
    version: 8,
    migrate: migratePersistedWorkstreamTabs,
  }),
);

export function isAgentTab(shell: ShellTab): boolean {
  return !shell.docPreviewPath && !shell.diffView && !shell.filePath && !shell.terminalView;
}

export function useAgentTabCount(workstreamId: string | null): number {
  return useWorkstreamTabsStore((s) =>
    workstreamId ? (s.byWs[workstreamId]?.shells.filter(isAgentTab).length ?? 0) : 0,
  );
}

export function updateWorkstreamTabs(
  next: (prev: Record<string, WsTabState>) => Record<string, WsTabState>,
): void {
  useWorkstreamTabsStore.setState((s) => ({ byWs: next(s.byWs) }));
}

export function seedWorkstreamMode(workstreamId: string, mode: StageKey): void {
  const base = defaultTabState();
  updateWorkstreamTabs((all) => ({ ...all, [workstreamId]: { ...base, shells: base.shells.map((s) => ({ ...s, mode })) } }));
}

export const useComposerFocusStore =create<{ pendingShellId: number | null }>(() => ({
  pendingShellId: null,
}));

export function requestComposerFocus(shellId: number): void {
  useComposerFocusStore.setState({ pendingShellId: shellId });
}

export function consumeComposerFocus(shellId: number): boolean {
  if (useComposerFocusStore.getState().pendingShellId !== shellId) return false;
  useComposerFocusStore.setState({ pendingShellId: null });
  return true;
}

export function bindSessionToShell(
  workstreamId: string,
  shellId: number,
  sessionId: string,
): void {
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId] ?? defaultTabState();
    if (!current.shells.some((s) => s.id === shellId)) return all;
    const bound = {
      ...current,
      shells: current.shells.map((s) => (s.id === shellId ? { ...s, sessionId } : s)),
    };
    return { ...all, [workstreamId]: focusShell(bound, shellId) };
  });
}

/** Records a mode the harness switched to itself, so a resume relaunches into it. */
export function setSessionPermissionMode(workstreamId: string, sessionId: string, key: string): void {
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId];
    if (!current?.shells.some((s) => s.sessionId === sessionId && s.permissionMode !== key)) return all;
    const shells = current.shells.map((s) => (s.sessionId === sessionId ? { ...s, permissionMode: key } : s));
    return { ...all, [workstreamId]: { ...current, shells } };
  });
}

/** The mode a session's tab would relaunch it in, for resumes that start outside the tab. */
export function sessionPermissionMode(workstreamId: string, sessionId: string): string | undefined {
  const shells = useWorkstreamTabsStore.getState().byWs[workstreamId]?.shells ?? [];
  return shells.find((s) => s.sessionId === sessionId)?.permissionMode;
}

const tabMode = (stage: StageKey): StageKey =>
  MODE_STAGES.some((s) => s.key === stage) ? stage : "general";

export function openSessionTab(workstreamId: string, session: Session): void {
  if (session.title != null) markTitleSeen(session.id, session.title);
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId] ?? defaultTabState();
    const open = current.shells.find((s) => s.sessionId === session.id);
    if (open) return { ...all, [workstreamId]: focusShell(current, open.id) };
    const parked = current.archived?.find((a) => a.shell.sessionId === session.id);
    if (parked) {
      const restored = unarchiveShellState(current, parked.shell.id);
      if (session.title == null) return { ...all, [workstreamId]: restored };
      // The server's title may be a rename made elsewhere after this tab was parked.
      const shells = restored.shells.map((s) =>
        s.id === parked.shell.id ? { ...s, title: session.title as string, titleLocked: true } : s,
      );
      return { ...all, [workstreamId]: { ...restored, shells } };
    }
    return {
      ...all,
      [workstreamId]: appendShell(current, {
        id: nextShellId(current.shells, current.archived),
        title: session.title ?? session.preview ?? stageLabel(session.stage),
        // Locked only for a server title, so a fallback never syncs back as one.
        titleLocked: session.title != null,
        mode: tabMode(session.stage),
        // The entry's stage must stay the server's, or a picked ship session is invisible to workstreamShipBusy.
        sessionStage: session.stage,
        harness: session.harness,
        draft: "",
        docs: [],
        attachments: [],
        sessionId: session.id,
        paneId: focusedPaneId(current),
      }),
    };
  });
}

export function openPromptTab(workstreamId: string, sessionId: string, title: string): void {
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId] ?? defaultTabState();
    return {
      ...all,
      [workstreamId]: appendShell(current, {
        id: nextShellId(current.shells, current.archived),
        title,
        mode: "general",
        draft: "",
        docs: [],
        attachments: [],
        sessionId,
        paneId: focusedPaneId(current),
        titleLocked: true,
      }),
    };
  });
}

export function openDocPreviewTab(workstreamId: string, path: string, title: string): void {
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId] ?? defaultTabState();
    const existing = current.shells.find((s) => s.docPreviewPath === path);
    return {
      ...all,
      [workstreamId]: existing
        ? focusShell(current, existing.id)
        : appendShell(current, {
            id: nextShellId(current.shells, current.archived),
            title,
            mode: "general",
            draft: "",
            docs: [],
            attachments: [],
            docPreviewPath: path,
            paneId: focusedPaneId(current),
          }),
    };
  });
}

function shellSeedFromDefaults(d?: ActionAgentDefaults): Partial<ShellTab> {
  if (!d) return {};
  const cleared = {
    model: undefined,
    effort: undefined,
    opencodeModel: undefined,
    codexModel: undefined,
    codexEffort: undefined,
  };
  if (d.harness === "claude") {
    return {
      ...cleared,
      harness: "claude",
      model: d.model ?? undefined,
      effort: d.effort ?? undefined,
    };
  }
  if (d.harness === "codex") {
    return {
      ...cleared,
      harness: "codex",
      codexModel: d.model ?? undefined,
      codexEffort: d.effort ?? undefined,
    };
  }
  return { ...cleared, harness: d.harness, opencodeModel: d.model ?? undefined };
}

export function openShipTab(
  workstreamId: string,
  sessionId: string,
  defaults?: ActionAgentDefaults,
): void {
  openDedicatedTab(workstreamId, sessionId, "ship", "Ship", defaults);
}

export function openCommitTab(
  workstreamId: string,
  sessionId: string,
  defaults?: ActionAgentDefaults,
): void {
  openDedicatedTab(workstreamId, sessionId, "commit", "Commit", defaults);
}

function openDedicatedTab(
  workstreamId: string,
  sessionId: string,
  mode: ShellTab["mode"],
  title: string,
  defaults?: ActionAgentDefaults,
): void {
  const seed = shellSeedFromDefaults(defaults);
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId] ?? defaultTabState();
    const existing = current.shells.find((s) => s.mode === mode);
    if (existing) {
      const reseeded = {
        ...current,
        shells: current.shells.map((s) =>
          s.id === existing.id ? { ...s, sessionId, ...seed } : s,
        ),
      };
      return { ...all, [workstreamId]: focusShell(reseeded, existing.id) };
    }
    return {
      ...all,
      [workstreamId]: appendShell(current, {
        id: nextShellId(current.shells, current.archived),
        title,
        mode,
        draft: "",
        docs: [],
        attachments: [],
        sessionId,
        paneId: focusedPaneId(current),
        ...seed,
      }),
    };
  });
}

export function openImplementTab(
  workstreamId: string,
  sessionId: string,
  planPath: string,
  from: ShellTab,
): void {
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId] ?? defaultTabState();
    return {
      ...all,
      [workstreamId]: appendShell(current, {
        id: nextShellId(current.shells, current.archived),
        title: "Implement",
        mode: "implement",
        draft: "",
        docs: [{ path: planPath, role: "plan" }],
        attachments: [],
        harness: from.harness,
        model: from.model,
        effort: from.effort,
        opencodeModel: from.opencodeModel,
        codexModel: from.codexModel,
        codexEffort: from.codexEffort,
        permissionMode: from.permissionMode,
        sessionId,
        paneId: focusedPaneId(current),
      }),
    };
  });
}
