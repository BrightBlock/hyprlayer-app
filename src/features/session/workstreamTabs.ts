
import { useLayoutEffect } from "react";
import { create } from "zustand";
import { persist } from "zustand/middleware";

import { scopedKey } from "@/lib/hostScope";
import type { Bootstrap, Harness, PermissionMode, Session } from "@/lib/bindings";
import { queryClient } from "@/lib/queryClient";
import { actionEngine, resolveMultiplexer } from "@/features/multiplexer/engines";

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
  /** Opened to read a session a goal check ended; viewing it never relaunches it. */
  frozen?: boolean;
  docPreviewPath?: string;
  diffView?: { path: string; staged: boolean; sha?: string };
  filePath?: string;
  terminalView?: { terminalId?: string };
  webView?: { url: string };
  ptyAgent?: {
    engineId: string;
    terminalId?: string;
    bypass?: boolean;
    sessionId?: string;
    started?: boolean;
    /** Spawned by an agent and shared by every desktop: closing the tab detaches, the agent keeps running. */
    adopted?: boolean;
    /** The app action it runs, so a second click focuses it rather than racing it. */
    action?: StageKey;
  };
  paneId: PaneId;
};

/** The model and effort a session launched with, as its tab stores them for that harness. */
export type LaunchedAgent = { model: string | null; effort: string | null };

export type WsTabState = {
  shells: ShellTab[];
  layout: LayoutNode;
  panes: Record<PaneId, { activeId: number }>;
  activePaneId: PaneId;
  archived?: ArchivedShell[];
  /** Agent-spawned sessions and PTYs this desktop has opened, so reconcile never reopens one the user closed. */
  agentTabsSeen?: string[];
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
  // Archiving ends a PTY, so a restore resumes it as this desktop's own rather than reattaching a dead shared one.
  const kept = shell.ptyAgent?.adopted ? { ...shell, ptyAgent: { ...shell.ptyAgent, adopted: undefined } } : shell;
  return { ...next, archived: [{ shell: kept, archivedAt, status }, ...(state.archived ?? [])] };
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
  return (
    !shell.docPreviewPath && !shell.diffView && !shell.filePath && !shell.terminalView && !shell.webView
  );
}

export function useAgentTabCount(workstreamId: string | null): number {
  return useWorkstreamTabsStore((s) =>
    workstreamId ? (s.byWs[workstreamId]?.shells.filter(isAgentTab).length ?? 0) : 0,
  );
}

export function updateWorkstreamTabs(
  next: (prev: Record<string, WsTabState>) => Record<string, WsTabState>,
): void {
  // persist serialises the whole store on every setState, even one that returns the same state.
  const prev = useWorkstreamTabsStore.getState().byWs;
  const byWs = next(prev);
  if (byWs !== prev) useWorkstreamTabsStore.setState({ byWs });
}

/** The tabs a workstream starts with: its first agent opens where app actions do. */
export function initialTabState(): WsTabState {
  const base = defaultTabState();
  const cfg = resolveMultiplexer(queryClient.getQueryData<Bootstrap>(["bootstrap"])?.settings);
  const engineId = actionEngine(cfg)?.id;
  if (!engineId) return base;
  return { ...base, shells: base.shells.map((s) => ({ ...s, ptyAgent: { engineId, started: false } })) };
}

/** For an update that brings the workstream's first agent tab itself, so no seeded one runs beside it. */
export const blankTabState = (): WsTabState => ({ ...defaultTabState(), shells: [], panes: { p1: { activeId: 0 } } });

/** Stores a workstream's first tabs; `mode` applies only to a native chat. */
export function seedWorkstreamTabs(workstreamId: string, mode?: StageKey): void {
  const base = initialTabState();
  const shells = base.shells.map((s) => (mode && !s.ptyAgent ? { ...s, mode } : s));
  updateWorkstreamTabs((all) => ({ ...all, [workstreamId]: { ...base, shells } }));
}

/**
 * Stores a viewed workstream's first tabs once settings are known, before paint, so no chat flashes first.
 * An agent-created workstream starts blank: its creator spawns the first agent into it.
 */
export function useSeedWorkstreamTabs(
  workstreamId: string | null,
  settingsLoaded: boolean,
  agentCreated: boolean,
): void {
  useLayoutEffect(() => {
    if (!settingsLoaded || !workstreamId || useWorkstreamTabsStore.getState().byWs[workstreamId]) return;
    if (agentCreated) updateWorkstreamTabs((all) => ({ ...all, [workstreamId]: blankTabState() }));
    else seedWorkstreamTabs(workstreamId);
  }, [workstreamId, settingsLoaded, agentCreated]);
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

/** `launched` is what the session started on, so a failed start's tab shows that, not an earlier pick. */
export function bindSessionToShell(
  workstreamId: string,
  shellId: number,
  sessionId: string,
  launched: { harness?: Harness; permissionMode?: PermissionMode | null } = {},
): void {
  const picks = {
    ...(launched.harness ? { harness: launched.harness } : {}),
    ...(launched.permissionMode ? { permissionMode: launched.permissionMode } : {}),
  };
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId] ?? initialTabState();
    if (!current.shells.some((s) => s.id === shellId)) return all;
    const bound = {
      ...current,
      shells: current.shells.map((s) => (s.id === shellId ? { ...s, sessionId, ...picks } : s)),
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

export function openSessionTab(
  workstreamId: string,
  session: Session,
  opts?: { frozen?: boolean; background?: boolean; permissionMode?: string; launched?: LaunchedAgent },
): void {
  if (session.title != null) markTitleSeen(session.id, session.title);
  const background = opts?.background === true;
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId] ?? blankTabState();
    const open = current.shells.find((s) => s.sessionId === session.id);
    if (open) return background ? all : { ...all, [workstreamId]: focusShell(current, open.id) };
    const parked = current.archived?.find((a) => a.shell.sessionId === session.id);
    if (parked && background) return all;
    if (parked) {
      const restored = unarchiveShellState(current, parked.shell.id);
      if (session.title == null) return { ...all, [workstreamId]: restored };
      // The server's title may be a rename made elsewhere after this tab was parked.
      const shells = restored.shells.map((s) =>
        s.id === parked.shell.id ? { ...s, title: session.title as string, titleLocked: true } : s,
      );
      return { ...all, [workstreamId]: { ...restored, shells } };
    }
    const shell: ShellTab = {
      id: nextShellId(current.shells, current.archived),
      title: session.title ?? session.preview ?? stageLabel(session.stage),
      // Locked only for a server title, so a fallback never syncs back as one.
      titleLocked: session.title != null,
      mode: tabMode(session.stage),
      // The entry's stage must stay the server's, or a picked ship session is invisible to workstreamShipBusy.
      sessionStage: session.stage,
      ...(opts?.frozen ? { frozen: true } : {}),
      ...(opts?.permissionMode ? { permissionMode: opts.permissionMode } : {}),
      ...(opts?.launched ? shellSeedFromDefaults({ harness: session.harness, ...opts.launched }) : {}),
      harness: session.harness,
      draft: "",
      docs: [],
      attachments: [],
      sessionId: session.id,
      paneId: focusedPaneId(current),
    };
    return {
      ...all,
      [workstreamId]: background
        ? { ...current, shells: [...current.shells, shell] }
        : appendShell(current, shell),
    };
  });
  if (session.origin === "agent") markAgentTabSeen(workstreamId, session.id);
}

const AGENT_TABS_SEEN_CAP = 200;

const agentTabSeen = (workstreamId: string, id: string): boolean =>
  (useWorkstreamTabsStore.getState().byWs[workstreamId]?.agentTabsSeen ?? []).includes(id);

function markAgentTabSeen(workstreamId: string, id: string): void {
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId] ?? initialTabState();
    if (current.agentTabsSeen?.includes(id)) return all;
    const agentTabsSeen = [...(current.agentTabsSeen ?? []), id].slice(-AGENT_TABS_SEEN_CAP);
    return { ...all, [workstreamId]: { ...current, agentTabsSeen } };
  });
}

/**
 * An agent-spawned session belongs to every desktop, so closing its tab only detaches it here.
 * `spawned` is the server's `origin === "agent"` rows, since `agentTabsSeen` evicts its oldest.
 */
export function isSpawnedSessionTab(
  state: WsTabState,
  shell: ShellTab,
  spawned?: readonly string[],
): boolean {
  if (!shell.sessionId) return false;
  return !!spawned?.includes(shell.sessionId) || !!state.agentTabsSeen?.includes(shell.sessionId);
}

/**
 * Opens an agent-spawned session in a background tab once per desktop; a closed one stays closed.
 * `permissionMode` pins what a relaunch from the tab asks for, so it never lands above the launch.
 */
export function openAgentSessionTab(
  workstreamId: string,
  session: Session,
  permissionMode: PermissionMode,
  launched?: LaunchedAgent,
): void {
  if (agentTabSeen(workstreamId, session.id)) return;
  openSessionTab(workstreamId, session, { background: true, permissionMode, launched });
  markAgentTabSeen(workstreamId, session.id);
}

/**
 * Adopts an agent-spawned PTY in a background tab once per desktop; it attaches when first shown.
 * `focus` is a user's pick from Agent sessions, which reattaches even a tab closed here.
 */
export function openAgentTerminalTab(
  workstreamId: string,
  pty: { terminalId: string; engineId: string; sessionId?: string; bypass: boolean },
  opts?: { focus?: boolean },
): void {
  const focus = opts?.focus === true;
  if (!focus && agentTabSeen(workstreamId, pty.terminalId)) return;
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId] ?? blankTabState();
    const open = current.shells.find((s) => s.ptyAgent?.terminalId === pty.terminalId);
    if (open) return focus ? { ...all, [workstreamId]: focusShell(current, open.id) } : all;
    const parked = focus && current.archived?.find((a) => a.shell.ptyAgent?.terminalId === pty.terminalId);
    if (parked && parked.shell.ptyAgent) {
      // Still running, so it is shared: closing it again must detach, never kill.
      const ptyAgent = { ...parked.shell.ptyAgent, started: true, adopted: true };
      const restored = unarchiveShellState(current, parked.shell.id);
      const shells = restored.shells.map((s) => (s.id === parked.shell.id ? { ...s, ptyAgent } : s));
      return { ...all, [workstreamId]: { ...restored, shells } };
    }
    const id = nextShellId(current.shells, current.archived);
    const shell: ShellTab = {
      id,
      title: `Agent ${id}`,
      mode: "general",
      draft: "",
      docs: [],
      attachments: [],
      paneId: focusedPaneId(current),
      ptyAgent: { ...pty, started: true, adopted: true },
    };
    return {
      ...all,
      [workstreamId]: focus ? appendShell(current, shell) : { ...current, shells: [...current.shells, shell] },
    };
  });
  markAgentTabSeen(workstreamId, pty.terminalId);
}

/** A PTY's new provider session (`/clear`, an in-TUI resume) reaches its tab even while the tab is unmounted. */
export function setPtyTabSessionId(workstreamId: string, terminalId: string, sessionId: string): void {
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId];
    const bound = (s: ShellTab) => s.ptyAgent?.terminalId === terminalId && s.ptyAgent.sessionId !== sessionId;
    if (!current?.shells.some(bound)) return all;
    const shells = current.shells.map((s) =>
      bound(s) && s.ptyAgent ? { ...s, ptyAgent: { ...s.ptyAgent, sessionId } } : s,
    );
    return { ...all, [workstreamId]: { ...current, shells } };
  });
}

/** A hooked agent's first prompt starts its tab even with no keystroke (agent or injected input). */
export function setPtyTabStarted(workstreamId: string, terminalId: string): void {
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId];
    const bound = (s: ShellTab) => s.ptyAgent?.terminalId === terminalId && !s.ptyAgent.started;
    if (!current?.shells.some(bound)) return all;
    const shells = current.shells.map((s) =>
      bound(s) && s.ptyAgent ? { ...s, ptyAgent: { ...s.ptyAgent, started: true } } : s,
    );
    return { ...all, [workstreamId]: { ...current, shells } };
  });
}

/**
 * Binds a tab to the terminal its agent now runs in, even while the tab is unmounted, and returns
 * the binding it replaced. A new terminal is this desktop's own launch, so the tab no longer adopts.
 */
export function bindPtyTabTerminal(
  workstreamId: string,
  shellId: number,
  terminalId: string,
  sessionId: string | undefined,
): ShellTab["ptyAgent"] {
  const prev = useWorkstreamTabsStore
    .getState()
    .byWs[workstreamId]?.shells.find((s) => s.id === shellId)?.ptyAgent;
  if (!prev || (prev.terminalId === terminalId && (!sessionId || prev.sessionId === sessionId))) {
    return prev;
  }
  const ptyAgent = {
    ...prev,
    terminalId,
    sessionId: sessionId ?? prev.sessionId,
    adopted: prev.terminalId === terminalId ? prev.adopted : undefined,
  };
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId];
    if (!current) return all;
    const shells = current.shells.map((s) => (s.id === shellId && s.ptyAgent ? { ...s, ptyAgent } : s));
    return { ...all, [workstreamId]: { ...current, shells } };
  });
  return prev;
}

type ViewerPatch = Pick<ShellTab, "docPreviewPath" | "filePath" | "diffView">;

/** An agent's Reveal: opens the viewer without taking focus; an open one is left as it is. */
export function openViewerTabInBackground(workstreamId: string, patch: ViewerPatch, title: string): void {
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId] ?? initialTabState();
    const open = current.shells.some(
      (s) =>
        (patch.docPreviewPath !== undefined && s.docPreviewPath === patch.docPreviewPath) ||
        (patch.filePath !== undefined && s.filePath === patch.filePath) ||
        (patch.diffView !== undefined &&
          s.diffView?.path === patch.diffView.path &&
          s.diffView.staged === patch.diffView.staged &&
          s.diffView.sha === undefined),
    );
    if (open) return all;
    const shell: ShellTab = {
      id: nextShellId(current.shells, current.archived),
      title,
      mode: "general",
      draft: "",
      docs: [],
      attachments: [],
      paneId: focusedPaneId(current),
      ...patch,
    };
    return { ...all, [workstreamId]: { ...current, shells: [...current.shells, shell] } };
  });
}

export function openPromptTab(workstreamId: string, sessionId: string, title: string): void {
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId] ?? blankTabState();
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
    const current = all[workstreamId] ?? initialTabState();
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
    const current = all[workstreamId] ?? blankTabState();
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

export function openPlanStageTab(
  workstreamId: string,
  sessionId: string,
  stage: "implement" | "validate",
  planPath: string,
  from: ShellTab | undefined,
): void {
  updateWorkstreamTabs((all) => {
    const current = all[workstreamId] ?? blankTabState();
    return {
      ...all,
      [workstreamId]: appendShell(current, {
        id: nextShellId(current.shells, current.archived),
        title: stageLabel(stage),
        mode: stage,
        draft: "",
        docs: [{ path: planPath, role: "plan" }],
        attachments: [],
        harness: from?.harness,
        model: from?.model,
        effort: from?.effort,
        opencodeModel: from?.opencodeModel,
        codexModel: from?.codexModel,
        codexEffort: from?.codexEffort,
        permissionMode: from?.permissionMode,
        sessionId,
        paneId: focusedPaneId(current),
      }),
    };
  });
}
