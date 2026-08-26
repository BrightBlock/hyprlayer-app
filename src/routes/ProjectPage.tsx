import { useNavigate, useParams, useSearch } from "@tanstack/react-router";
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";

import {
  Button,
  ContextMenu,
  ContextMenuItem,
  Dialog,
  IconButton,
  MenuSeparator,
  PageError,
  PageSpinner,
  TextInput,
} from "@/components/ui";
import { open } from "@tauri-apps/plugin-dialog";

import { ArchiveIcon, DiffIcon, DocIcon, PlusIcon, SplitIcon, TerminalIcon, XIcon } from "@/components/icons";
import { ScrollableTabStrip } from "@/components/ScrollableTabs";
import { commands, type WorkstreamView } from "@/lib/bindings";
import { GLASS_BAND } from "@/lib/glass";
import { CreateWorkstreamDialog } from "@/features/workstreams/CreateWorkstreamDialog";
import { ChangesPanel } from "@/features/workstreams/ChangesPanel";
import { GitPanel } from "@/features/workstreams/GitPanel";
import { PrChecksPanel } from "@/features/workstreams/PrChecksPanel";
import { DiffTab } from "@/features/workstreams/DiffTab";
import { FileViewerTab } from "@/features/workstreams/FileViewerTab";
import { SessionChat, SessionTabDot, SessionTabIcon } from "@/features/session/SessionChat";
import { ArchivedAgentsView } from "@/features/session/ArchivedAgentsView";
import { DocPreviewTab } from "@/features/session/DocPreviewTab";
import { TabTitle } from "@/features/session/TabTitle";
import { TerminalTab } from "@/features/session/TerminalTab";
import { SidebarTasksPanel } from "@/features/session/PlanTasksView";
import { AgentPtyTab } from "@/features/multiplexer/AgentPtyTab";
import { EngineGlyph, PtyTabDot } from "@/features/multiplexer/agentStateUi";
import {
  closeAgent,
  ensureAgent,
  resolveResume,
  setAllNotifyOnBlocked,
} from "@/features/multiplexer/agentRuntime";
import type { AgentState } from "@/features/multiplexer/engineState";
import { agentKey, useMultiplexerStore } from "@/features/multiplexer/multiplexerStore";
import {
  defaultEngine,
  enabledEngines,
  engineMintsViaExec,
  engineOrFallback,
  mintPtySessionId,
  relaunchArgs,
  useMultiplexer,
  type ResolvedEngine,
} from "@/features/multiplexer/engines";
import { ScriptDock } from "@/features/session/ScriptDock";
import { requestRun, setActiveWorkstream, useScriptsStore } from "@/features/session/scriptsStore";
import { openProjectSettings } from "@/features/projects/projectSettingsStore";
import { useTabDrag, type TabDragHandlers } from "@/features/session/useTabDrag";
import {
  dropSession,
  killSession,
  stopSession,
  transcriptToText,
  useSession,
  useSessionStore,
} from "@/features/session/sessionStore";
import { isBusy, isTerminal, type SessionState } from "@/features/session/transcript";
import { useCommand } from "@/lib/keybindings/registry";
import { TAB_SELECT_COUNT, tabSelectId } from "@/lib/keybindings/commands";
import {
  importDroppedPaths,
  IMAGE_EXTENSIONS,
  type Attachment,
} from "@/features/session/attachments";
import { isGitProject } from "@/lib/domain/projects";
import { nextModeStage, stageLabel, type StageKey } from "@/lib/domain/stages";
import {
  archiveShellState,
  clearArchiveOnRefocus,
  cycleActiveTab,
  defaultTabState,
  nextShellId,
  removeShellState,
  requestComposerFocus,
  selectLastTab,
  selectTabAt,
  unarchiveShellState,
  updateWorkstreamTabs,
  useWorkstreamTabsStore,
  type ArchivedStatus,
  type ShellTab,
  type WsTabState,
} from "@/features/session/workstreamTabs";
import { WorkstreamDocsPanel } from "@/features/workstreams/WorkstreamDocsPanel";
import { chipLabel } from "@/features/workstreams/WorkstreamDocChips";
import { ProjectBoard } from "@/features/board/ProjectBoard";
import { AttachPrDialog } from "@/features/workstreams/AttachPrDialog";
import { useBootstrap, useDetachPr, useSetWorkstreamStage, useWorkstreams } from "@/lib/queries";
import { basename } from "@/lib/path";
import { shortSha } from "@/lib/format";
import { drawerWidth, splitPaneFloor } from "@/lib/layout";
import { clamp, startEdgeDrag } from "@/lib/resize";
import { useElementWidth } from "@/lib/useElementWidth";
import { useViewport } from "@/lib/useViewport";
import { DIFF_SIDEBAR_MAX, DIFF_SIDEBAR_MIN, useUiStore } from "@/lib/uiStore";

type ShellMenu = {
  shellId: number;
  x: number;
  y: number;
};

function isViewerShell(shell: ShellTab): boolean {
  return !!(shell.docPreviewPath || shell.diffView || shell.filePath);
}

function isTerminalShell(shell: ShellTab): boolean {
  return !!shell.terminalView;
}

function isPtyShell(shell: ShellTab): boolean {
  return !!shell.ptyAgent;
}

function isArchivable(shell: ShellTab): boolean {
  return isAgentShell(shell) || isPtyShell(shell);
}

function isAgentShell(shell: ShellTab): boolean {
  return !isViewerShell(shell) && !isTerminalShell(shell) && !isPtyShell(shell);
}

const AGENT_BUSY_HINT = "Agent is still working. Stop it first.";

function classifyArchivedStatus(state: SessionState | undefined): ArchivedStatus {
  if (!state) return "idle";
  if (state === "completed") return "done";
  if (isTerminal(state)) return "stopped";
  return "idle";
}

function classifyPtyArchivedStatus(state: AgentState | undefined): ArchivedStatus {
  return state === "done" ? "done" : "idle";
}

function TabCloseControl({
  shell,
  onArchive,
  onClose,
}: {
  shell: ShellTab;
  onArchive: () => void;
  onClose: () => void;
}) {
  const session = useSession(shell.sessionId);
  const archivable = isArchivable(shell);
  const busy = isAgentShell(shell) && session ? isBusy(session) : false;
  const label = archivable ? (busy ? AGENT_BUSY_HINT : `Archive ${shell.title}`) : `Close ${shell.title}`;
  return (
    <button
      type="button"
      aria-disabled={busy}
      onClick={(e) => {
        e.stopPropagation();
        if (busy) return;
        if (archivable) onArchive();
        else onClose();
      }}
      className={`mr-1 inline-flex h-5 w-5 items-center justify-center rounded ${
        busy ? "cursor-not-allowed text-ink-dim/40" : "text-ink-dim hover:bg-edge hover:text-ink"
      }`}
      title={label}
      aria-label={label}
    >
      <XIcon />
    </button>
  );
}

function detachDoc(shell: ShellTab, path: string): ShellTab {
  const removed = shell.docs.find((d) => d.path === path);
  const docs = shell.docs.filter((d) => d.path !== path);
  const persistDismissal = Boolean(removed?.suggested) || removed?.role === "handoff";
  if (!persistDismissal) return { ...shell, docs };
  return {
    ...shell,
    docs,
    dismissedSuggestions: [...new Set([...(shell.dismissedSuggestions ?? []), path])],
  };
}

function TabSelectCommand({ index, onSelect }: { index: number; onSelect: (i: number) => void }) {
  useCommand(tabSelectId(index), () => onSelect(index));
  return null;
}

function ProjectCommands(props: {
  sessionId: string | undefined;
  onNewTab: () => void;
  onArchiveTab: (() => void) | null;
  onCloseTab: (() => void) | null;
  onReopenTab: (() => void) | null;
  onSelectTab: (index: number) => void;
  onSelectLastTab: () => void;
  onToggleSplit: () => void;
  onNextTab: () => void;
  onPrevTab: () => void;
  onNewWorkstream: () => void;
  onToggleRun: (() => void) | null;
  onCycleStage: (() => void) | null;
  onToggleArchived: (() => void) | null;
}) {
  const session = useSession(props.sessionId);
  const running = Boolean(props.sessionId && session && isBusy(session));
  const sid = props.sessionId;
  useCommand("tab.new", props.onNewTab);
  useCommand("tab.archive", props.onArchiveTab);
  useCommand("tab.close", props.onCloseTab);
  useCommand("tab.reopen", props.onReopenTab);
  useCommand("tab.selectLast", props.onSelectLastTab);
  useCommand("tab.toggleSplit", props.onToggleSplit);
  useCommand("tab.next", props.onNextTab);
  useCommand("tab.prev", props.onPrevTab);
  useCommand("ws.new", props.onNewWorkstream);
  useCommand("scripts.run", props.onToggleRun);
  useCommand("ws.cycleStage", props.onCycleStage);
  useCommand("view.toggleArchivedAgents", props.onToggleArchived);
  useCommand("session.stop", running && sid ? () => void stopSession(sid) : null);
  useCommand("session.clear", sid ? () => void killSession(sid) : null);
  return (
    <>
      {Array.from({ length: TAB_SELECT_COUNT }, (_, i) => (
        <TabSelectCommand key={i} index={i} onSelect={props.onSelectTab} />
      ))}
    </>
  );
}

export function ProjectPage() {
  const { projectPath } = useParams({ from: "/projects/$projectPath" });
  const bootstrap = useBootstrap();
  const workstreams = useWorkstreams(projectPath);
  const setStage = useSetWorkstreamStage(projectPath);

  const search = useSearch({ from: "/projects/$projectPath" });
  const navigate = useNavigate({ from: "/projects/$projectPath" });

  const selectedId = search.item;

  const tabsByWs = useWorkstreamTabsStore((s) => s.byWs);
  const views = (workstreams.data ?? []).filter((view) => view.item.status === "active");
  const activeView = selectedId ? (views.find((v) => v.item.id === selectedId) ?? null) : null;
  const wsId = activeView?.item.id ?? null;
  const multiplexer = useMultiplexer();
  const dockOpen = useScriptsStore((s) => (wsId ? !!s.dock[wsId]?.open : false));
  const dockPosition = useScriptsStore((s) => s.dockPosition);
  const [shellMenu, setShellMenu] = useState<ShellMenu | null>(null);
  const [newAgentMenu, setNewAgentMenu] = useState<{
    x: number;
    y: number;
    pane: "main" | "right";
  } | null>(null);
  const [createOpen, setCreateOpen] = useState(false);
  const [cardMenu, setCardMenu] = useState<{ x: number; y: number; view: WorkstreamView } | null>(
    null,
  );
  const [attachPrTarget, setAttachPrTarget] = useState<WorkstreamView | null>(null);
  const detachPr = useDetachPr();
  const [renameShellId, setRenameShellId] = useState<number | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [docsModalShellId, setDocsModalShellId] = useState<number | null>(null);
  const { narrow } = useViewport();
  const { ref: paneRef, width: paneWidth } = useElementWidth<HTMLDivElement>();
  const narrowDrawer = useUiStore((state) => state.narrowDrawer);
  const diffHidden = useUiStore((state) => state.diffHidden);
  const diffWidth = useUiStore((state) => state.diffWidth);
  const setDiffWidth = useUiStore((state) => state.setDiffWidth);
  const [resizingDiff, setResizingDiff] = useState(false);
  const [rightRatio, setRightRatio] = useState(0.5);
  const [maximized, setMaximized] = useState<{ wsId: string; pane: "main" | "right" } | null>(
    null,
  );
  const splitRowRef = useRef<HTMLDivElement>(null);
  const dragHandlersRef = useRef<TabDragHandlers>({ reorder: () => {}, move: () => {} });
  const { drag, dropPane, suppressClickRef, startDrag } = useTabDrag(dragHandlersRef);

  useEffect(() => {
    if (search.create) {
      setCreateOpen(true);
      void navigate({
        search: (prev) => ({ ...prev, create: undefined }),
        replace: true,
      });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [search.create]);

  const rightDock = !!activeView && dockOpen && dockPosition === "right";
  const showAside = narrow ? narrowDrawer === "diff" : !diffHidden;
  const asideWidth = narrow ? drawerWidth(diffWidth, paneWidth) : diffWidth;
  const showBoard = selectedId === undefined;

  useEffect(() => {
    if (selectedId === undefined) return;
    if (!workstreams.isSuccess || workstreams.isFetching) return;
    if (!views.some((v) => v.item.id === selectedId)) {
      void navigate({ search: (prev) => ({ ...prev, item: undefined }), replace: true });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedId, workstreams.isSuccess, workstreams.isFetching]);


  const ptyAgentSig =
    multiplexer.enabled && wsId
      ? (tabsByWs[wsId]?.shells ?? [])
          .filter((s) => s.ptyAgent)
          .map((s) => `${s.id}:${s.ptyAgent!.engineId}`)
          .join(",")
      : "";
  useEffect(() => {
    setAllNotifyOnBlocked(multiplexer.notifyOnBlocked);
  }, [multiplexer.notifyOnBlocked]);

  useEffect(() => {
    if (!multiplexer.enabled || !multiplexer.persistAcrossRestarts) return;
    if (!wsId || !activeView) return;
    for (const s of tabsByWs[wsId]?.shells ?? []) {
      if (!s.ptyAgent) continue;
      if (engineMintsViaExec(s.ptyAgent.engineId) && !s.ptyAgent.sessionId) continue;
      const base = engineOrFallback(multiplexer, s.ptyAgent.engineId);
      const bypass = !!s.ptyAgent.bypass;
      const pty = s.ptyAgent;
      void (async () => {
        const fallback = pty.started ?? !!pty.terminalId;
        const resume = await resolveResume(pty.engineId, pty.sessionId, wsId, fallback);
        void ensureAgent({
          workstreamId: wsId,
          shellId: s.id,
          engine: { ...base, args: relaunchArgs(base, { sessionId: pty.sessionId, resume, bypass }) },
          projectPath,
          workstreamTitle: activeView.item.title,
          persistedTerminalId: pty.terminalId,
          notifyOnBlocked: multiplexer.notifyOnBlocked,
          bypass,
          started: resume,
        });
      })();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ptyAgentSig, wsId, multiplexer.enabled, multiplexer.persistAcrossRestarts, multiplexer.notifyOnBlocked]);

  useEffect(() => {
    setActiveWorkstream(wsId);
    return () => setActiveWorkstream(null);
  }, [wsId]);

  useEffect(() => {
    if (!wsId) return;
    const tabs = tabsByWs[wsId] ?? defaultTabState();
    const pane = tabs.activePane ?? "main";
    const inPane = (s: ShellTab) =>
      pane === "right" ? s.panel === "right" : (s.panel ?? "main") !== "right";
    const members = tabs.shells.filter(inPane);
    const activeId = pane === "right" ? tabs.rightActiveId : tabs.activeId;
    const focused = members.find((s) => s.id === activeId) ?? members[0] ?? null;
    if (focused && isAgentShell(focused)) requestComposerFocus(focused.id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wsId]);

  const updateTabs = useCallback(
    (updater: (current: WsTabState) => WsTabState) => {
      if (!wsId) return;
      updateWorkstreamTabs((all) => {
        const current = all[wsId] ?? defaultTabState();
        return { ...all, [wsId]: clearArchiveOnRefocus(current, updater(current)) };
      });
    },
    [wsId],
  );
  const openViewerTab = useCallback(
    (match: (shell: ShellTab) => boolean, title: string, fields: Partial<ShellTab>) => {
      updateTabs((current) => {
        const existing = current.shells.find(match);
        if (existing) return { ...current, activeId: existing.id, activePane: "main" };
        const nextId = nextShellId(current.shells, current.archived);
        const shell: ShellTab = {
          id: nextId,
          title,
          mode: "general",
          draft: "",
          docs: [],
          attachments: [],
          panel: "main",
          ...fields,
        };
        return {
          ...current,
          shells: [...current.shells, shell],
          activeId: nextId,
          activePane: "main",
        };
      });
    },
    [updateTabs],
  );
  const openDocPreview = useCallback(
    (path: string, title: string) =>
      openViewerTab((s) => s.docPreviewPath === path, title, { docPreviewPath: path }),
    [openViewerTab],
  );
  const openDiff = useCallback(
    (path: string, staged: boolean, sha?: string) =>
      openViewerTab(
        (s) => s.diffView?.path === path && s.diffView?.staged === staged && s.diffView?.sha === sha,
        sha ? `${basename(path)} @ ${shortSha(sha)}` : basename(path),
        { diffView: { path, staged, sha } },
      ),
    [openViewerTab],
  );
  const openFile = useCallback(
    (path: string) => openViewerTab((s) => s.filePath === path, basename(path), { filePath: path }),
    [openViewerTab],
  );

  if (bootstrap.isLoading || workstreams.isLoading) return <PageSpinner />;
  if (bootstrap.isError) return <PageError error={bootstrap.error} />;
  if (workstreams.isError) return <PageError error={workstreams.error} />;

  const project = bootstrap.data!.projects.find((p) => p.path === projectPath);
  if (!project) {
    return (
      <PageError
        error={{
          code: "project_not_found",
          message: `No hyprlayer project is mapped at ${projectPath}`,
          remedy: "Add the repository as a project first.",
        }}
      />
    );
  }

  const folderProject = !isGitProject(project);

  const tabs = wsId ? (tabsByWs[wsId] ?? defaultTabState()) : null;
  const shells = tabs?.shells ?? [];
  const isMainShell = (s: ShellTab) => (s.panel ?? "main") !== "right";
  const isRightShell = (s: ShellTab) => s.panel === "right";
  const mainShells = shells.filter(isMainShell);
  const rightShells = shells.filter(isRightShell);
  const rightSplit = (tabs?.rightSplit ?? false) && rightShells.length > 0;
  const activePane: "main" | "right" = tabs?.activePane ?? "main";
  const maxPane = maximized && maximized.wsId === wsId ? maximized.pane : null;
  const maxMain = maxPane === "main" && rightSplit;
  const maxRight = maxPane === "right" && rightSplit;
  const maximizedView = maxMain || maxRight;
  const activeShell = tabs
    ? (mainShells.find((s) => s.id === tabs.activeId) ?? mainShells[0] ?? null)
    : null;
  const rightActiveShell =
    rightShells.find((s) => s.id === tabs?.rightActiveId) ?? rightShells[0] ?? null;
  const focusedShell = activePane === "right" ? rightActiveShell : activeShell;
  const docsModalShell =
    docsModalShellId !== null ? (shells.find((s) => s.id === docsModalShellId) ?? null) : null;
  const contextShell = shellMenu ? shells.find((s) => s.id === shellMenu.shellId) : null;
  const archivedShells = tabs?.archived ?? [];
  const ptySessionIds = [...shells, ...archivedShells.map((a) => a.shell)]
    .map((s) => s.ptyAgent?.sessionId)
    .filter((id): id is string => !!id);
  const archiveViewOpen = tabs?.archiveOpen ?? false;
  const toggleArchived = wsId
    ? () => updateTabs((c) => ({ ...c, archiveOpen: !c.archiveOpen }))
    : null;
  const contextIsLastMain = !!contextShell && isMainShell(contextShell) && mainShells.length === 1;
  const contextEntry = contextShell?.sessionId
    ? useSessionStore.getState().sessions[contextShell.sessionId]
    : undefined;
  const archiveBlockedReason =
    contextEntry && isBusy(contextEntry)
      ? AGENT_BUSY_HINT
      : contextIsLastMain
        ? "Keep at least one agent in the strip."
        : null;
  const shellBeingRenamed = renameShellId
    ? shells.find((s) => s.id === renameShellId)
    : null;

  const modifyShell = (shellId: number, transform: (shell: ShellTab) => ShellTab) => {
    updateTabs((current) => ({
      ...current,
      shells: current.shells.map((s) => (s.id === shellId ? transform(s) : s)),
    }));
  };
  const patchShell = (
    shellId: number,
    patch: Partial<
      Pick<
        ShellTab,
        | "title"
        | "titleLocked"
        | "mode"
        | "draft"
        | "docs"
        | "attachments"
        | "model"
        | "effort"
        | "permissionMode"
        | "harness"
        | "opencodeModel"
        | "codexModel"
        | "codexEffort"
        | "sessionId"
        | "handoffNudgeDismissed"
        | "terminalView"
        | "ptyAgent"
      >
    >,
  ) => modifyShell(shellId, (s) => ({ ...s, ...patch }));
  const setTerminalId = (shellId: number, terminalId: string) =>
    patchShell(shellId, { terminalView: { terminalId } });
  const setPtyTerminalId = (shellId: number, engineId: string, terminalId: string) =>
    modifyShell(shellId, (s) => ({ ...s, ptyAgent: { ...s.ptyAgent, engineId, terminalId } }));
  const setPtyBypass = (shellId: number, bypass: boolean) =>
    modifyShell(shellId, (s) => (s.ptyAgent ? { ...s, ptyAgent: { ...s.ptyAgent, bypass } } : s));
  const setPtyStarted = (shellId: number) =>
    modifyShell(shellId, (s) =>
      s.ptyAgent && !s.ptyAgent.started ? { ...s, ptyAgent: { ...s.ptyAgent, started: true } } : s,
    );
  const setPtySessionId = (shellId: number, sessionId: string) =>
    modifyShell(shellId, (s) =>
      s.ptyAgent && s.ptyAgent.sessionId !== sessionId
        ? { ...s, ptyAgent: { ...s.ptyAgent, sessionId } }
        : s,
    );
  const selectShell = (shellId: number) => {
    updateTabs((current) => ({ ...current, activeId: shellId, activePane: "main" }));
  };
  const selectRightShell = (shellId: number) => {
    updateTabs((current) => ({ ...current, rightActiveId: shellId, activePane: "right" }));
  };
  const setActivePane = (pane: "main" | "right") => {
    updateTabs((current) => (current.activePane === pane ? current : { ...current, activePane: pane }));
  };
  const cycleTab = (dir: 1 | -1) => updateTabs((current) => cycleActiveTab(current, dir));
  const makeShell = (id: number, panel: "main" | "right", ptyEngineId?: string): ShellTab => ({
    id,
    title: `Agent ${id}`,
    mode: "general",
    draft: "",
    docs: [],
    attachments: [],
    panel,
    ...(ptyEngineId
      ? {
          ptyAgent: { engineId: ptyEngineId, sessionId: mintPtySessionId(ptyEngineId), started: false },
        }
      : {}),
  });
  const spawnRightShell = (current: WsTabState, ptyEngineId?: string): WsTabState => {
    const id = nextShellId(current.shells, current.archived);
    return {
      ...current,
      shells: [...current.shells, makeShell(id, "right", ptyEngineId)],
      rightActiveId: id,
      rightSplit: true,
      activePane: "right",
    };
  };
  const addShell = (ptyEngineId?: string) => {
    updateTabs((current) => {
      const id = nextShellId(current.shells, current.archived);
      return {
        ...current,
        shells: [...current.shells, makeShell(id, "main", ptyEngineId)],
        activeId: id,
        activePane: "main",
      };
    });
  };
  const addRightShell = (ptyEngineId?: string) => updateTabs((c) => spawnRightShell(c, ptyEngineId));
  const onNewTabDefault = () => {
    if (!multiplexer.enabled) return addShell();
    addShell(defaultEngine(multiplexer)?.id);
  };
  const openNewAgentMenu = (e: ReactMouseEvent, pane: "main" | "right") => {
    e.stopPropagation();
    const r = e.currentTarget.getBoundingClientRect();
    setNewAgentMenu({ x: r.left, y: r.bottom + 4, pane });
  };
  const onPlus = (e: ReactMouseEvent, pane: "main" | "right") => {
    if (!multiplexer.enabled) return pane === "main" ? addShell() : addRightShell();
    openNewAgentMenu(e, pane);
  };
  const ptyEngineFor = (engineId: string): ResolvedEngine => engineOrFallback(multiplexer, engineId);
  const isLivePty = (shell: ShellTab): boolean => multiplexer.enabled && !!shell.ptyAgent;
  const toggleRightSplit = () => {
    setMaximized(null);
    updateTabs((current) => {
      if (current.rightSplit) return { ...current, rightSplit: false, activePane: "main" };
      if (current.shells.some(isRightShell)) {
        return { ...current, rightSplit: true, activePane: "right" };
      }
      return spawnRightShell(current);
    });
  };
  const closeRightSplit = () => {
    setMaximized(null);
    for (const s of rightShells) {
      if (s.sessionId) void killSession(s.sessionId);
      if (s.terminalView?.terminalId) void commands.terminalClose(s.terminalView.terminalId);
      if (s.ptyAgent && wsId) closeAgent(wsId, s.id);
    }
    updateTabs((current) => ({
      ...current,
      shells: current.shells.filter(isMainShell),
      rightSplit: false,
      rightActiveId: undefined,
      activePane: "main",
    }));
    for (const s of rightShells) if (s.sessionId) dropSession(s.sessionId);
  };
  const moveShellToPane = (shellId: number, toPane: "main" | "right") => {
    updateTabs((current) => {
      const shell = current.shells.find((s) => s.id === shellId);
      if (!shell || (shell.panel ?? "main") === toPane) return current;
      if (toPane === "right") {
        const mains = current.shells.filter(isMainShell);
        if (mains.length <= 1) return current;
        return {
          ...current,
          shells: current.shells.map((s) =>
            s.id === shellId ? { ...s, panel: "right" as const } : s,
          ),
          activeId:
            current.activeId === shellId
              ? (mains.find((m) => m.id !== shellId)?.id ?? current.activeId)
              : current.activeId,
          rightSplit: true,
          rightActiveId: shellId,
          activePane: "right",
        };
      }
      const shellsNext = current.shells.map((s) =>
        s.id === shellId ? { ...s, panel: "main" as const } : s,
      );
      const stillRight = shellsNext.some(isRightShell);
      return {
        ...current,
        shells: shellsNext,
        activeId: shellId,
        activePane: "main",
        rightSplit: stillRight ? current.rightSplit : false,
        rightActiveId:
          current.rightActiveId === shellId
            ? shellsNext.find(isRightShell)?.id
            : current.rightActiveId,
      };
    });
  };
  const reorderShellInPane = (shellId: number, pane: "main" | "right", toIndex: number) => {
    updateTabs((current) => {
      const inPane = (s: ShellTab) =>
        pane === "right" ? s.panel === "right" : (s.panel ?? "main") !== "right";
      const members = current.shells.filter(inPane);
      const from = members.findIndex((s) => s.id === shellId);
      if (from < 0) return current;
      const reordered = [...members];
      const [moved] = reordered.splice(from, 1);
      reordered.splice(clamp(toIndex, 0, reordered.length), 0, moved);
      if (reordered.every((s, i) => s.id === members[i].id)) return current;
      let i = 0;
      return { ...current, shells: current.shells.map((s) => (inPane(s) ? reordered[i++] : s)) };
    });
  };
  dragHandlersRef.current = { reorder: reorderShellInPane, move: moveShellToPane };
  const closeShell = (shellId: number) => {
    if (!tabs) return;
    const shell = tabs.shells.find((s) => s.id === shellId);
    if (!shell) return;
    if (isMainShell(shell) && tabs.shells.filter(isMainShell).length === 1) return;
    if (shell.sessionId) void killSession(shell.sessionId);
    if (shell.terminalView?.terminalId) void commands.terminalClose(shell.terminalView.terminalId);
    if (shell.ptyAgent && wsId) closeAgent(wsId, shellId);
    updateTabs((current) => removeShellState(current, shellId));
    if (shell.sessionId) dropSession(shell.sessionId);
  };
  const archiveShell = (shellId: number) => {
    const shell = shells.find((s) => s.id === shellId);
    if (!shell) return;
    let status: ArchivedStatus;
    if (shell.ptyAgent) {
      const ptyState = wsId
        ? useMultiplexerStore.getState().agents[agentKey(wsId, shellId)]?.state
        : undefined;
      status = classifyPtyArchivedStatus(ptyState);
      if (wsId) closeAgent(wsId, shellId);
    } else {
      const entry = shell.sessionId
        ? useSessionStore.getState().sessions[shell.sessionId]
        : undefined;
      status = classifyArchivedStatus(entry?.state);
    }
    updateTabs((current) => archiveShellState(current, shellId, status, Date.now()));
    if (!shell.ptyAgent && shell.sessionId) {
      const entry = useSessionStore.getState().sessions[shell.sessionId];
      if (entry && !entry.live) dropSession(shell.sessionId);
    }
  };
  const unarchiveShell = (shellId: number) => {
    updateTabs((current) => unarchiveShellState(current, shellId));
    requestComposerFocus(shellId);
  };
  const archiveOrCloseShell = (shellId: number) => {
    const shell = shells.find((s) => s.id === shellId);
    if (!shell) return;
    if (!isArchivable(shell)) {
      closeShell(shellId);
      return;
    }
    const session = isAgentShell(shell) && shell.sessionId
      ? useSessionStore.getState().sessions[shell.sessionId]
      : undefined;
    if (session && isBusy(session)) return;
    archiveShell(shellId);
  };
  const reopenLastClosedShell = () => {
    const newest = tabs?.archived?.[0];
    if (newest) unarchiveShell(newest.shell.id);
  };
  const removeArchivedShell = (shellId: number) => {
    const sessionId = tabs?.archived?.find((a) => a.shell.id === shellId)?.shell.sessionId;
    updateTabs((current) => ({
      ...current,
      archived: (current.archived ?? []).filter((a) => a.shell.id !== shellId),
    }));
    if (sessionId) dropSession(sessionId);
  };
  const openDoc = (id: string) => {
    void navigate({ to: "/library", search: { doc: id } });
  };
  const attachDocToShell = (path: string, _role: string) => {
    if (!docsModalShell) return;
    addAttachmentsToShell(docsModalShell.id, [{ path, name: chipLabel(path), kind: "file" }]);
    modifyShell(docsModalShell.id, (s) => {
      const docs = s.docs.filter((d) => !(d.suggested && d.path === path));
      return docs.length === s.docs.length ? s : { ...s, docs };
    });
  };
  const detachDocFromShell = (path: string) => {
    if (!docsModalShell) return;
    removeAttachmentFromShell(docsModalShell.id, path);
  };

  const selectItem = (id: string) => {
    void navigate({
      search: (prev) => ({ ...prev, item: id }),
      replace: true,
    });
  };
  const changeShellMode = (shell: ShellTab, key: StageKey) => {
    if (!activeView || key === shell.mode) return;
    patchShell(shell.id, { mode: key });
    setStage.mutate({ id: activeView.item.id, stage: key });
  };
  const addAttachmentsToShell = (shellId: number, atts: Attachment[]) => {
    if (atts.length === 0) return;
    modifyShell(shellId, (s) => {
      const existing = s.attachments ?? [];
      const have = new Set(existing.map((a) => a.path));
      const added = atts.filter((a) => !have.has(a.path));
      return added.length ? { ...s, attachments: [...existing, ...added] } : s;
    });
  };
  const removeAttachmentFromShell = (shellId: number, path: string) =>
    modifyShell(shellId, (s) => ({
      ...s,
      attachments: (s.attachments ?? []).filter((a) => a.path !== path),
    }));
  const clearShellAttachments = (shellId: number) => patchShell(shellId, { attachments: [] });
  const addFilesToShell = async (shellId: number, imagesOnly: boolean) => {
    if (!wsId) return;
    const selected = await open({
      multiple: true,
      directory: false,
      title: imagesOnly ? "Add screenshots" : "Add files",
      filters: imagesOnly ? [{ name: "Images", extensions: IMAGE_EXTENSIONS }] : undefined,
    });
    if (!selected) return;
    const paths = Array.isArray(selected) ? selected : [selected];
    const atts = await importDroppedPaths(wsId, paths);
    addAttachmentsToShell(shellId, atts);
  };
  const suggestDocToShell = (shellId: number, path: string | null, role: string) =>
    modifyShell(shellId, (s) => {
      const dismissed = s.dismissedSuggestions ?? [];
      const manual = s.docs.filter((d) => !d.suggested);
      const staged = (s.attachments ?? []).some((a) => a.path === path);
      const alreadyManual = path != null && (staged || manual.some((d) => d.path === path));
      const keepSuggested =
        path && !alreadyManual && !dismissed.includes(path)
          ? [{ path, role, suggested: true as const }]
          : [];
      return { ...s, docs: [...manual, ...keepSuggested] };
    });

  const shellChatProps = (shell: ShellTab) => ({
    view: activeView!,
    shell,
    onDraftChange: (value: string) => patchShell(shell.id, { draft: value }),
    onModeChange: (key: StageKey) => changeShellMode(shell, key),
    onModelChange: (key: string) => patchShell(shell.id, { model: key }),
    onEffortChange: (key: string) => patchShell(shell.id, { effort: key }),
    onPermissionModeChange: (key: string) => patchShell(shell.id, { permissionMode: key }),
    onHarnessChange: (key: string) => patchShell(shell.id, { harness: key }),
    onOpencodeModelChange: (key: string) => {
      patchShell(shell.id, { opencodeModel: key });
      if (shell.sessionId) void commands.setSessionModel(shell.sessionId, key);
    },
    onCodexModelChange: (key: string) => {
      patchShell(shell.id, { codexModel: key });
      if (shell.sessionId) void commands.setSessionModel(shell.sessionId, key);
    },
    onCodexEffortChange: (key: string) => {
      patchShell(shell.id, { codexEffort: key });
      if (shell.sessionId) void commands.setSessionEffort(shell.sessionId, key);
    },
    onOpenHarnessSettings: () => void navigate({ to: "/settings", search: { section: "harnesses" } }),
    onSessionStarted: (sessionId: string) =>
      modifyShell(shell.id, (s) => ({
        ...s,
        sessionId,
        handoffNudgeDismissed: false,
        docs: s.docs.filter((d) => d.role !== "handoff"),
      })),
    onResetSession: () => patchShell(shell.id, { sessionId: undefined }),
    onOpenDoc: openDoc,
    onDetachDoc: (path: string) => modifyShell(shell.id, (s) => detachDoc(s, path)),
    onSuggestDoc: (path: string | null, role: string) => suggestDocToShell(shell.id, path, role),
    onDismissHandoffNudge: () => patchShell(shell.id, { handoffNudgeDismissed: true }),
    onAttachHandoffDoc: (path: string) =>
      modifyShell(shell.id, (s) =>
        s.docs.some((d) => d.path === path)
          ? s
          : { ...s, docs: [...s.docs, { path, role: "handoff" }] },
      ),
    onBrowseDocs: folderProject ? null : () => setDocsModalShellId(shell.id),
    onAddFiles: () => void addFilesToShell(shell.id, false),
    onAddScreenshots: () => void addFilesToShell(shell.id, true),
    onAddAttachments: (atts: Attachment[]) => addAttachmentsToShell(shell.id, atts),
    onRemoveAttachment: (path: string) => removeAttachmentFromShell(shell.id, path),
    onClearAttachments: () => clearShellAttachments(shell.id),
  });
  const openRenameShell = (shell: ShellTab) => {
    setRenameShellId(shell.id);
    setRenameValue(shell.title);
  };
  const closeRenameShell = () => {
    setRenameShellId(null);
    setRenameValue("");
  };
  const commitRenameShell = () => {
    if (!shellBeingRenamed) return;
    const nextTitle = renameValue.trim();
    if (!nextTitle) return;
    patchShell(shellBeingRenamed.id, { title: nextTitle, titleLocked: true });
    closeRenameShell();
  };
  const copyShellTranscript = (shell: ShellTab) => {
    const body = shell.sessionId
      ? transcriptToText(shell.sessionId)
      : `Draft (no session started yet):\n${shell.draft || "(empty)"}`;
    const transcript = [
      `# ${activeView?.item.title ?? project.mappedName} · ${shell.title}`,
      `Project: ${project.mappedName}`,
      `Branch: ${activeView?.item.branch ?? "—"}`,
      `Mode: ${stageLabel(shell.mode)}`,
      "",
      body,
      "",
    ].join("\n");
    void navigator.clipboard.writeText(transcript);
  };
  const startSplitResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    const rowW = splitRowRef.current?.clientWidth ?? 800;
    const floor = splitPaneFloor(rowW);
    startEdgeDrag(event, {
      startWidth: rightRatio * rowW,
      min: floor,
      max: Math.max(floor, rowW - floor),
      direction: -1,
      onResize: (paneWidth) => setRightRatio(clamp(paneWidth / rowW, 0.2, 0.8)),
    });
  };
  const startDiffResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    startEdgeDrag(event, {
      startWidth: diffWidth,
      min: DIFF_SIDEBAR_MIN,
      max: DIFF_SIDEBAR_MAX,
      direction: -1,
      onResize: setDiffWidth,
      onStart: () => setResizingDiff(true),
      onEnd: () => setResizingDiff(false),
    });
  };

  const renderTabPill = (
    shell: ShellTab,
    opts: {
      active: boolean;
      paneFocused: boolean;
      pane: "main" | "right";
      onSelect: () => void;
      closable: boolean;
    },
  ) => (
    <div
      key={shell.id}
      data-tab-id={shell.id}
      role="tab"
      aria-selected={opts.active}
      onPointerDown={(e) =>
        startDrag(e, { id: shell.id, title: shell.title, fromPane: opts.pane })
      }
      onClick={() => {
        if (!suppressClickRef.current) opts.onSelect();
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        opts.onSelect();
        setShellMenu({ shellId: shell.id, x: e.clientX, y: e.clientY });
      }}
      className={`inline-flex shrink-0 cursor-pointer select-none items-center rounded-md ${
        drag?.id === shell.id ? "opacity-60 bg-accent/10 ring-2 ring-inset ring-accent" : ""
      } ${
        opts.active
          ? opts.paneFocused
            ? "bg-panel-2 text-ink"
            : "bg-panel-2/50 text-ink-dim"
          : "text-ink-dim hover:bg-panel-2 hover:text-ink"
      }`}
    >
      <span className="flex min-w-0 items-center gap-1.5 px-3 py-1.5 text-sm font-medium">
        {shell.diffView ? (
          <span className="rounded bg-accent/15 p-0.5 text-accent" title="Diff view">
            <DiffIcon size={13} />
          </span>
        ) : isTerminalShell(shell) ? (
          <span className="text-ink-dim">
            <TerminalIcon size={13} />
          </span>
        ) : isLivePty(shell) && shell.ptyAgent ? (
          (() => {
            const e = ptyEngineFor(shell.ptyAgent.engineId);
            return (
              <EngineGlyph
                glyph={e.glyph}
                accentToken={e.accentToken}
                className="text-[11px] leading-none"
              />
            );
          })()
        ) : isViewerShell(shell) ? (
          <span className="text-ink-dim">
            <DocIcon size={13} />
          </span>
        ) : (
          <SessionTabIcon mode={shell.mode} sessionId={shell.sessionId} />
        )}
        <span className="max-w-[160px] truncate" title={shell.title}>
          <TabTitle workstreamId={wsId ?? ""} shellId={shell.id} title={shell.title} />
        </span>
        {isLivePty(shell) ? (
          <PtyTabDot workstreamId={wsId ?? ""} shellId={shell.id} />
        ) : isAgentShell(shell) ? (
          <SessionTabDot sessionId={shell.sessionId} />
        ) : null}
      </span>
      {opts.closable && (
        <TabCloseControl
          shell={shell}
          onArchive={() => archiveShell(shell.id)}
          onClose={() => closeShell(shell.id)}
        />
      )}
    </div>
  );

  const renderPaneContent = (shell: ShellTab | null) => {
    if (!activeView || !shell) {
      return (
        <div className="flex min-h-0 flex-1 items-center justify-center p-6 text-sm text-ink-dim">
          No agent in this pane
        </div>
      );
    }
    if (shell.docPreviewPath) {
      return (
        <DocPreviewTab
          key={`${activeView.item.id}:${shell.id}`}
          path={shell.docPreviewPath}
          title={shell.title}
        />
      );
    }
    if (shell.diffView) {
      return (
        <DiffTab
          key={`${activeView.item.id}:${shell.id}`}
          id={activeView.item.id}
          path={shell.diffView.path}
          staged={shell.diffView.staged}
          sha={shell.diffView.sha}
        />
      );
    }
    if (shell.filePath) {
      return (
        <FileViewerTab
          key={`${activeView.item.id}:${shell.id}`}
          id={activeView.item.id}
          path={shell.filePath}
        />
      );
    }
    if (shell.terminalView) {
      return (
        <TerminalTab
          key={`${activeView.item.id}:${shell.id}`}
          workstreamId={activeView.item.id}
          shellId={shell.id}
          terminalId={shell.terminalView.terminalId}
          autoFocus={shell.id === focusedShell?.id}
          onTerminalId={(tid) => setTerminalId(shell.id, tid)}
        />
      );
    }
    if (isLivePty(shell) && shell.ptyAgent) {
      const engine = ptyEngineFor(shell.ptyAgent.engineId);
      return (
        <AgentPtyTab
          key={`${activeView.item.id}:${shell.id}`}
          workstreamId={activeView.item.id}
          shellId={shell.id}
          engine={engine}
          projectPath={projectPath}
          workstreamTitle={activeView.item.title}
          terminalId={shell.ptyAgent.terminalId}
          sessionId={shell.ptyAgent.sessionId}
          started={shell.ptyAgent.started}
          bypass={shell.ptyAgent.bypass}
          autoFocus={shell.id === focusedShell?.id}
          excludeSessionIds={ptySessionIds}
          onTerminalId={(tid) => setPtyTerminalId(shell.id, engine.id, tid)}
          onBypassChange={(b) => setPtyBypass(shell.id, b)}
          onStarted={() => setPtyStarted(shell.id)}
          onSessionId={(sid) => setPtySessionId(shell.id, sid)}
          notifyOnBlocked={multiplexer.notifyOnBlocked}
        />
      );
    }
    return <SessionChat key={`${activeView.item.id}:${shell.id}`} {...shellChatProps(shell)} />;
  };

  if (showBoard) {
    return (
      <>
        <ProjectBoard
          views={views}
          folder={folderProject}
          onOpen={(id) => selectItem(id)}
          onNewWorkstream={() => setCreateOpen(true)}
          onCardContextMenu={
            folderProject ? undefined : (view, x, y) => setCardMenu({ x, y, view })
          }
        />
        {cardMenu && (
          <ContextMenu position={cardMenu} onClose={() => setCardMenu(null)}>
            <ContextMenuItem
              onClick={() => {
                setAttachPrTarget(cardMenu.view);
                setCardMenu(null);
              }}
            >
              {cardMenu.view.item.prNumber === null ? "Attach PR…" : "Change PR…"}
            </ContextMenuItem>
            {cardMenu.view.item.prNumber !== null && (
              <ContextMenuItem
                onClick={() => {
                  detachPr.mutate(cardMenu.view.item.id);
                  setCardMenu(null);
                }}
              >
                Detach PR #{cardMenu.view.item.prNumber}
              </ContextMenuItem>
            )}
          </ContextMenu>
        )}
        <AttachPrDialog target={attachPrTarget} onClose={() => setAttachPrTarget(null)} />
        <CreateWorkstreamDialog
          projectPath={projectPath}
          folder={folderProject}
          open={createOpen}
          onClose={() => setCreateOpen(false)}
          onCreated={(view) => selectItem(view.item.id)}
        />
      </>
    );
  }

  if (selectedId && !activeView && workstreams.isFetching) return <PageSpinner />;

  return (
    <div ref={paneRef} className="relative flex h-full min-h-0 bg-surface">
      <ProjectCommands
        sessionId={focusedShell?.sessionId}
        onNewTab={onNewTabDefault}
        onArchiveTab={focusedShell ? () => archiveOrCloseShell(focusedShell.id) : null}
        onCloseTab={focusedShell ? () => closeShell(focusedShell.id) : null}
        onReopenTab={tabs?.archived?.length ? reopenLastClosedShell : null}
        onSelectTab={(index) => updateTabs((current) => selectTabAt(current, index))}
        onSelectLastTab={() => updateTabs(selectLastTab)}
        onToggleSplit={toggleRightSplit}
        onNextTab={() => cycleTab(1)}
        onPrevTab={() => cycleTab(-1)}
        onNewWorkstream={() => setCreateOpen(true)}
        onToggleRun={
          wsId
            ? () => {
                const id = wsId;
                if (useScriptsStore.getState().running[id]) void commands.scriptStop(id);
                else requestRun(id);
              }
            : null
        }
        onCycleStage={
          focusedShell && isAgentShell(focusedShell) && !focusedShell.sessionId
            ? () => changeShellMode(focusedShell, nextModeStage(focusedShell.mode))
            : null
        }
        onToggleArchived={toggleArchived}
      />
      <section className="flex min-w-0 flex-1 flex-col">
        <div className="flex min-h-0 flex-1">
          <main className="flex min-w-0 flex-1 flex-col">
            <div
              data-glass
              className={`relative z-10 flex h-9 items-stretch border-b border-edge ${GLASS_BAND}`}
            >
              <div
                data-pane="main"
                className={`flex min-w-0 flex-1 items-center justify-between gap-3 pl-5 pr-2.5 ${
                  maxRight ? "hidden" : ""
                } ${dropPane === "main" ? "bg-accent/10" : ""}`}
              >
                <ScrollableTabStrip
                  activeId={activeShell?.id ?? null}
                  trailing={
                    activeView ? (
                      <IconButton
                        onClick={(e) => onPlus(e, "main")}
                        title="New agent"
                        aria-label="New agent"
                      >
                        <PlusIcon />
                      </IconButton>
                    ) : null
                  }
                >
                  {activeView &&
                    mainShells.map((shell) =>
                      renderTabPill(shell, {
                        active: shell.id === activeShell?.id,
                        paneFocused: !rightSplit || activePane === "main",
                        pane: "main",
                        onSelect: () => selectShell(shell.id),
                        closable: mainShells.length > 1,
                      }),
                    )}
                </ScrollableTabStrip>
                {activeView && (
                  <div className="flex shrink-0 items-center gap-1">
                    <button
                      type="button"
                      onClick={() => toggleArchived?.()}
                      title="Archived agents"
                      aria-label="Archived agents"
                      aria-pressed={archiveViewOpen}
                      className={`inline-flex h-8 items-center gap-1 rounded-md px-2 transition-colors hover:bg-panel-2 hover:text-ink ${
                        archiveViewOpen ? "bg-panel-2 text-ink" : "text-ink-dim"
                      }`}
                    >
                      <ArchiveIcon size={15} />
                      {archivedShells.length > 0 && (
                        <span className="font-mono text-[11px] leading-none">
                          {archivedShells.length}
                        </span>
                      )}
                    </button>
                    <IconButton
                      onClick={toggleRightSplit}
                      active={rightSplit}
                      title={rightSplit ? "Close split" : "Split agent right"}
                      aria-label={rightSplit ? "Close split" : "Split agent right"}
                    >
                      <SplitIcon className={rightSplit ? "text-accent" : undefined} />
                    </IconButton>
                  </div>
                )}
              </div>
              {activeView && rightSplit && !archiveViewOpen && (
                <div
                  data-pane="right"
                  className={`flex shrink-0 items-center gap-1 border-l border-edge px-3 ${
                    maxMain ? "hidden" : ""
                  } ${dropPane === "right" ? "bg-accent/10" : ""}`}
                  style={{ width: maxRight ? "100%" : `${rightRatio * 100}%` }}
                >
                  <ScrollableTabStrip
                    grow
                    activeId={rightActiveShell?.id ?? null}
                    trailing={
                      <IconButton
                        onClick={(e) => onPlus(e, "right")}
                        title="New agent"
                        aria-label="New agent"
                      >
                        <PlusIcon />
                      </IconButton>
                    }
                  >
                    {rightShells.map((shell) =>
                      renderTabPill(shell, {
                        active: shell.id === rightActiveShell?.id,
                        paneFocused: activePane === "right",
                        pane: "right",
                        onSelect: () => selectRightShell(shell.id),
                        closable: true,
                      }),
                    )}
                  </ScrollableTabStrip>
                  <IconButton onClick={closeRightSplit} title="Close split" aria-label="Close split">
                    <XIcon />
                  </IconButton>
                </div>
              )}
            </div>
            {shellMenu && contextShell && (
              <ContextMenu position={shellMenu} onClose={() => setShellMenu(null)}>
                <ContextMenuItem
                  onClick={() => {
                    openRenameShell(contextShell);
                    setShellMenu(null);
                  }}
                >
                  Rename tab
                </ContextMenuItem>
                <ContextMenuItem
                  onClick={() => {
                    copyShellTranscript(contextShell);
                    setShellMenu(null);
                  }}
                >
                  Copy transcript
                </ContextMenuItem>
                {contextShell.panel === "right" ? (
                  <ContextMenuItem
                    onClick={() => {
                      moveShellToPane(contextShell.id, "main");
                      setShellMenu(null);
                    }}
                  >
                    Move to main pane
                  </ContextMenuItem>
                ) : (
                  <ContextMenuItem
                    disabled={mainShells.length <= 1}
                    onClick={() => {
                      moveShellToPane(contextShell.id, "right");
                      setShellMenu(null);
                    }}
                  >
                    Move to right split
                  </ContextMenuItem>
                )}
                {rightSplit &&
                  (maxPane === (contextShell.panel ?? "main") ? (
                    <ContextMenuItem
                      onClick={() => {
                        setMaximized(null);
                        setShellMenu(null);
                      }}
                    >
                      Restore split view
                    </ContextMenuItem>
                  ) : isTerminalShell(contextShell) ? (
                    <ContextMenuItem
                      onClick={() => {
                        const pane = contextShell.panel ?? "main";
                        if (pane === "right") selectRightShell(contextShell.id);
                        else selectShell(contextShell.id);
                        if (wsId) setMaximized({ wsId, pane });
                        setShellMenu(null);
                      }}
                    >
                      Expand to full view
                    </ContextMenuItem>
                  ) : null)}
                {isArchivable(contextShell) && (
                  <ContextMenuItem
                    disabled={archiveBlockedReason !== null}
                    hint={archiveBlockedReason ?? undefined}
                    onClick={() => {
                      archiveShell(contextShell.id);
                      setShellMenu(null);
                    }}
                  >
                    Archive agent
                  </ContextMenuItem>
                )}
                <ContextMenuItem
                  danger={isArchivable(contextShell)}
                  disabled={contextIsLastMain}
                  onClick={() => {
                    closeShell(contextShell.id);
                    setShellMenu(null);
                  }}
                >
                  {isArchivable(contextShell) ? "Close agent" : "Close tab"}
                </ContextMenuItem>
              </ContextMenu>
            )}
            {newAgentMenu &&
              (() => {
                const menu = newAgentMenu;
                const def = defaultEngine(multiplexer);
                const spawn = (engineId?: string) =>
                  menu.pane === "main" ? addShell(engineId) : addRightShell(engineId);
                return (
                  <ContextMenu position={menu} onClose={() => setNewAgentMenu(null)}>
                    <div className="px-2 pb-1 pt-0.5 text-xs text-ink-dim">
                      New agent in {activeView?.item.title ?? "workstream"}
                    </div>
                    {enabledEngines(multiplexer).map((engine) => (
                      <ContextMenuItem
                        key={engine.id}
                        onClick={() => {
                          spawn(engine.id);
                          setNewAgentMenu(null);
                        }}
                      >
                        <span className="flex w-full items-center gap-2">
                          <EngineGlyph
                            glyph={engine.glyph}
                            accentToken={engine.accentToken}
                            className="text-[13px] leading-none"
                          />
                          <span className="min-w-0 flex-1 truncate">{engine.label}</span>
                          {engine.id === def?.id && (
                            <span className="font-mono text-[11px] text-ink-dim">⌘T</span>
                          )}
                        </span>
                      </ContextMenuItem>
                    ))}
                    <MenuSeparator />
                    <ContextMenuItem
                      onClick={() => {
                        spawn();
                        setNewAgentMenu(null);
                      }}
                    >
                      <span className="flex w-full items-center gap-2">
                        <SessionTabIcon mode="general" />
                        <span className="min-w-0 flex-1 truncate">Native chat</span>
                      </span>
                    </ContextMenuItem>
                    <MenuSeparator />
                    <ContextMenuItem
                      onClick={() => {
                        setNewAgentMenu(null);
                        void navigate({ to: "/settings", search: { section: "advanced" } });
                      }}
                    >
                      Configure engines…
                    </ContextMenuItem>
                  </ContextMenu>
                );
              })()}
            <Dialog open={Boolean(shellBeingRenamed)} onClose={closeRenameShell} title="Rename tab">
              <form
                className="space-y-3"
                onSubmit={(e) => {
                  e.preventDefault();
                  commitRenameShell();
                }}
              >
                <TextInput
                  value={renameValue}
                  onChange={(e) => setRenameValue(e.target.value)}
                  autoFocus
                />
                <div className="flex justify-end gap-2">
                  <Button type="button" onClick={closeRenameShell}>
                    Cancel
                  </Button>
                  <Button type="submit" variant="primary" disabled={!renameValue.trim()}>
                    Rename
                  </Button>
                </div>
              </form>
            </Dialog>
            <CreateWorkstreamDialog
              projectPath={projectPath}
              folder={folderProject}
              open={createOpen}
              onClose={() => setCreateOpen(false)}
              onCreated={(view) => selectItem(view.item.id)}
            />

            {activeView ? (
              archiveViewOpen ? (
                <ArchivedAgentsView
                  workstreamTitle={activeView.item.title}
                  branch={activeView.item.branch}
                  archived={archivedShells}
                  onUnarchive={unarchiveShell}
                  onRemove={removeArchivedShell}
                />
              ) : (
                <div ref={splitRowRef} className="flex min-h-0 flex-1">
                  <div
                    data-pane="main"
                    className={`relative flex min-h-0 min-w-0 flex-col ${
                      maxRight ? "hidden" : "flex-1"
                    } ${dropPane === "main" ? "ring-2 ring-inset ring-accent/60" : ""}`}
                    onPointerDownCapture={() => {
                      if (rightSplit && activePane !== "main") setActivePane("main");
                    }}
                  >
                    {renderPaneContent(activeShell)}
                  </div>
                  {rightSplit && (
                    <>
                      <div
                        role="separator"
                        aria-orientation="vertical"
                        aria-label="Resize agent split"
                        className={`relative z-10 -mx-1 w-2 shrink-0 cursor-col-resize bg-transparent transition-colors hover:bg-edge/60 ${
                          maximizedView ? "hidden" : ""
                        }`}
                        onPointerDown={startSplitResize}
                      />
                      <div
                        data-pane="right"
                        className={`relative flex min-h-0 flex-col border-l border-edge ${
                          maxMain ? "hidden" : ""
                        } ${maxRight ? "flex-1" : ""} ${
                          dropPane === "right" ? "ring-2 ring-inset ring-accent/60" : ""
                        }`}
                        style={maxRight ? undefined : { width: `${rightRatio * 100}%` }}
                        onPointerDownCapture={() => {
                          if (activePane !== "right") setActivePane("right");
                        }}
                      >
                        {renderPaneContent(rightActiveShell)}
                      </div>
                    </>
                  )}
                </div>
              )
            ) : (
              <div className="min-h-0 flex-1 overflow-y-auto p-5">
                <div className="mx-auto max-w-3xl space-y-4">
                  <div className="rounded-lg border border-edge bg-panel p-6 text-center">
                    <p className="text-base font-medium text-ink">Create your first workstream</p>
                    <p className="mx-auto mt-1 max-w-md text-sm leading-relaxed text-ink-dim">
                      {folderProject
                        ? "Each workstream is its own conversation, all running in this folder. Start as many as you need."
                        : "Each workstream gets its own branch and worktree, so agents can research, plan, and implement without touching your checkout."}
                    </p>
                    <Button variant="primary" className="mt-4" onClick={() => setCreateOpen(true)}>
                      <PlusIcon />
                      New workstream
                    </Button>
                  </div>
                </div>
              </div>
            )}
            {activeView && dockOpen && dockPosition === "bottom" && (
              <ScriptDock
                key={activeView.item.id}
                workstreamId={activeView.item.id}
                projectPath={projectPath}
                onEditScripts={() => openProjectSettings(projectPath)}
              />
            )}
          </main>
        </div>
      </section>

      {showAside && !narrow && (
        <div
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize diff sidebar"
          className="relative z-10 -mx-1 w-2 shrink-0 cursor-col-resize bg-transparent transition-colors hover:bg-edge/60"
          onPointerDown={startDiffResize}
        />
      )}
      <aside
        className={`shrink-0 overflow-hidden border-l border-edge bg-panel ${
          narrow ? "absolute inset-y-0 right-0 z-30 shadow-lg" : "relative"
        } ${resizingDiff ? "" : "transition-[width] duration-200 ease-out"}`}
        style={{ width: showAside ? asideWidth : 0 }}
        inert={!showAside}
      >
        <div
          className="absolute inset-y-0 right-0 flex flex-col"
          style={{ width: asideWidth }}
        >
          {!folderProject && <GitPanel view={activeView ?? null} />}
          <div className="flex min-h-0 flex-1 flex-col">
            <ChangesPanel
              workstreamId={activeView?.item.id ?? null}
              onOpenDoc={openDocPreview}
              onOpenDiff={openDiff}
              onOpenFile={openFile}
              filesOnly={folderProject}
              baseBranch={activeView?.item.baseBranch ?? null}
            />
          </div>
          <SidebarTasksPanel sessionId={focusedShell?.sessionId} />
          {rightDock && showAside && (
            <ScriptDock
              key={`${activeView!.item.id}:right`}
              workstreamId={activeView!.item.id}
              projectPath={projectPath}
              onEditScripts={() => openProjectSettings(projectPath)}
              orientation="right"
            />
          )}
          {!folderProject && <PrChecksPanel view={activeView ?? null} />}
        </div>
      </aside>

      <Dialog
        open={docsModalShellId !== null}
        onClose={() => setDocsModalShellId(null)}
        title="Attach context from library"
        size="2xl"
      >
        <WorkstreamDocsPanel
          thoughtsProject={folderProject ? null : project.thoughtsProject}
          targetTitle={docsModalShell?.title ?? null}
          attachedPaths={(docsModalShell?.attachments ?? []).map((a) => a.path)}
          onAttach={attachDocToShell}
          onDetach={detachDocFromShell}
          onOpenDoc={openDoc}
        />
      </Dialog>

      {drag && (
        <div
          className="pointer-events-none fixed z-50 scale-105 rounded-md border border-accent/70 bg-panel-2 px-2.5 py-1.5 text-xs font-medium text-ink shadow-2xl ring-2 ring-accent/30"
          style={{ left: drag.x + 12, top: drag.y + 12 }}
        >
          {drag.title}
        </div>
      )}
    </div>
  );
}
