
import { create } from "zustand";
import { persist } from "zustand/middleware";

import { scopedKey } from "@/lib/hostScope";

import type { Attachment } from "./attachments";
import { debouncedJSONStorage } from "@/lib/persistedStorage";
import {
  isSupportedOpencodeModel,
  type ActionAgentDefaults,
} from "@/lib/domain/harnessing";
import type { StageKey } from "@/lib/domain/stages";

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
  panel?: "main" | "right";
};

export type WsTabState = {
  shells: ShellTab[];
  activeId: number;
  rightSplit?: boolean;
  rightActiveId?: number;
  activePane?: "main" | "right";
  archived?: ArchivedShell[];
  archiveOpen?: boolean;
};

export type ArchivedStatus = "done" | "stopped" | "idle";

export type ArchivedShell = {
  shell: ShellTab;
  archivedAt: number;
  status: ArchivedStatus;
};

export const defaultTabState = (): WsTabState => ({
  shells: [
    { id: 1, title: "Agent", mode: "general", draft: "", docs: [], attachments: [], panel: "main" },
  ],
  activeId: 1,
});

export const nextShellId = (shells: ShellTab[], archived: ArchivedShell[] = []): number =>
  Math.max(0, ...shells.map((s) => s.id), ...archived.map((a) => a.shell.id)) + 1;

const paneOf = (s: ShellTab): "main" | "right" => (s.panel === "right" ? "right" : "main");

function focusedPane(state: WsTabState): { pane: "main" | "right"; members: ShellTab[] } {
  const pane = state.activePane ?? "main";
  return { pane, members: state.shells.filter((s) => paneOf(s) === pane) };
}

function focusShell(state: WsTabState, pane: "main" | "right", id: number): WsTabState {
  return pane === "right"
    ? { ...state, rightActiveId: id, activePane: "right" }
    : { ...state, activeId: id, activePane: "main" };
}

export function cycleActiveTab(state: WsTabState, dir: 1 | -1): WsTabState {
  const { pane, members } = focusedPane(state);
  if (members.length < 2) return state;
  const activeId = pane === "right" ? state.rightActiveId : state.activeId;
  const from = Math.max(0, members.findIndex((s) => s.id === activeId));
  return focusShell(state, pane, members[(from + dir + members.length) % members.length].id);
}

export function selectTabAt(state: WsTabState, index: number): WsTabState {
  const { pane, members } = focusedPane(state);
  const target = members[index];
  return target ? focusShell(state, pane, target.id) : state;
}

export function selectLastTab(state: WsTabState): WsTabState {
  const { pane, members } = focusedPane(state);
  const target = members[members.length - 1];
  return target ? focusShell(state, pane, target.id) : state;
}

export function removeShellState(state: WsTabState, shellId: number): WsTabState {
  const shell = state.shells.find((s) => s.id === shellId);
  if (!shell) return state;
  const pane = paneOf(shell);
  const onRight = pane === "right";
  if (!onRight && state.shells.filter((s) => paneOf(s) === "main").length <= 1) return state;

  const before = state.shells.filter((s) => paneOf(s) === pane);
  const closedIndex = before.findIndex((s) => s.id === shellId);
  const after = before.filter((s) => s.id !== shellId);
  const successor = after[Math.min(closedIndex, after.length - 1)]?.id;

  const remaining = state.shells.filter((s) => s.id !== shellId);
  if (onRight) {
    return {
      ...state,
      shells: remaining,
      rightActiveId: state.rightActiveId === shellId ? successor : state.rightActiveId,
      rightSplit: after.length > 0 ? state.rightSplit : false,
      activePane: after.length > 0 ? state.activePane : "main",
    };
  }
  return {
    ...state,
    shells: remaining,
    activeId: state.activeId === shellId ? (successor ?? state.activeId) : state.activeId,
  };
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
  const restored: ShellTab = { ...entry.shell, panel: "main" };
  return {
    ...state,
    shells: [...state.shells, restored],
    archived: archived.filter((a) => a.shell.id !== shellId),
    activeId: restored.id,
    activePane: "main",
  };
}

export function clearArchiveOnRefocus(prev: WsTabState, next: WsTabState): WsTabState {
  const refocused =
    next.shells.length >= prev.shells.length &&
    (next.activeId !== prev.activeId ||
      next.activePane !== prev.activePane ||
      next.rightActiveId !== prev.rightActiveId);
  return refocused && next.archiveOpen ? { ...next, archiveOpen: false } : next;
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

export function migratePersistedWorkstreamTabs(persisted: unknown): TabsStore {
  const state = persisted as TabsStore | undefined;
  if (!state?.byWs) return { byWs: {} };
  for (const ws of Object.values(state.byWs)) {
    if (!ws) continue;
    ws.shells = (ws.shells ?? []).filter((shell) => !!shell).map(normalizePersistedShell);
    ws.archived = (ws.archived ?? [])
      .filter((entry) => !!entry?.shell)
      .map((entry) => ({ ...entry, shell: normalizePersistedShell(entry.shell) }));
  }
  return state;
}

export const useWorkstreamTabsStore = create<TabsStore>()(
  persist(() => ({ byWs: {} }), {
    name: scopedKey("hyprlayer:workstream-tabs"),
    storage: debouncedJSONStorage(),
    version: 6,
    migrate: migratePersistedWorkstreamTabs,
  }),
);

export function updateWorkstreamTabs(
  next: (prev: Record<string, WsTabState>) => Record<string, WsTabState>,
): void {
  useWorkstreamTabsStore.setState((s) => ({ byWs: next(s.byWs) }));
}

export const useComposerFocusStore = create<{ pendingShellId: number | null }>(() => ({
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
    const shell = current.shells.find((s) => s.id === shellId);
    if (!shell) return all;
    const inRightPane = shell.panel === "right";
    return {
      ...all,
      [workstreamId]: {
        ...current,
        shells: current.shells.map((s) => (s.id === shellId ? { ...s, sessionId } : s)),
        activeId: inRightPane ? current.activeId : shellId,
        rightActiveId: inRightPane ? shellId : current.rightActiveId,
        rightSplit: inRightPane ? true : current.rightSplit,
        activePane: inRightPane ? "right" : "main",
        archiveOpen: false,
      },
    };
  });
}

export function openPromptTab(workstreamId: string, sessionId: string, title: string): void {
  updateWorkstreamTabs((all) => {
    const current = { ...(all[workstreamId] ?? defaultTabState()), archiveOpen: false };
    const nextId = nextShellId(current.shells, current.archived);
    return {
      ...all,
      [workstreamId]: {
        ...current,
        shells: [
          ...current.shells,
          {
            id: nextId,
            title,
            mode: "general",
            draft: "",
            docs: [],
            attachments: [],
            sessionId,
            panel: "main",
            titleLocked: true,
          },
        ],
        activeId: nextId,
        activePane: "main",
      },
    };
  });
}

export function openDocPreviewTab(workstreamId: string, path: string, title: string): void {
  updateWorkstreamTabs((all) => {
    const current = { ...(all[workstreamId] ?? defaultTabState()), archiveOpen: false };
    const existing = current.shells.find((s) => s.docPreviewPath === path);
    if (existing) {
      const inRightPane = existing.panel === "right";
      return {
        ...all,
        [workstreamId]: {
          ...current,
          ...(inRightPane
            ? { rightActiveId: existing.id, rightSplit: true }
            : { activeId: existing.id }),
          activePane: inRightPane ? "right" : "main",
        },
      };
    }
    const nextId = nextShellId(current.shells, current.archived);
    return {
      ...all,
      [workstreamId]: {
        ...current,
        shells: [
          ...current.shells,
          {
            id: nextId,
            title,
            mode: "general",
            draft: "",
            docs: [],
            attachments: [],
            docPreviewPath: path,
            panel: "main",
          },
        ],
        activeId: nextId,
        activePane: "main",
      },
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
      const inRightPane = existing.panel === "right";
      return {
        ...all,
        [workstreamId]: {
          ...current,
          shells: current.shells.map((s) =>
            s.id === existing.id ? { ...s, sessionId, ...seed } : s,
          ),
          activeId: inRightPane ? current.activeId : existing.id,
          rightActiveId: inRightPane ? existing.id : current.rightActiveId,
          rightSplit: inRightPane ? true : current.rightSplit,
          activePane: inRightPane ? "right" : "main",
        },
      };
    }
    const nextId = nextShellId(current.shells, current.archived);
    return {
      ...all,
      [workstreamId]: {
        ...current,
        shells: [
          ...current.shells,
          {
            id: nextId,
            title,
            mode,
            draft: "",
            docs: [],
            attachments: [],
            sessionId,
            panel: "main",
            ...seed,
          },
        ],
        activeId: nextId,
        activePane: "main",
      },
    };
  });
}
