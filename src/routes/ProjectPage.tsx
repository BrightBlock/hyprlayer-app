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
  SegmentedControl,
  TextInput,
} from "@/components/ui";
import { open } from "@tauri-apps/plugin-dialog";

import { Icon } from "@/components/Icon";
import { ScrollableTabStrip } from "@/components/ScrollableTabs";
import { commands, type WorkstreamView } from "@/lib/bindings";
import { GLASS_BAND } from "@/lib/glass";
import { openGoalSheet } from "@/lib/goalSheet";
import { CreateWorkstreamDialog } from "@/features/workstreams/CreateWorkstreamDialog";
import { ChangesPanel } from "@/features/workstreams/ChangesPanel";
import { GitPanel } from "@/features/workstreams/GitPanel";
import { PrChecksPanel } from "@/features/workstreams/PrChecksPanel";
import { DiffTab } from "@/features/workstreams/DiffTab";
import { FileViewerTab } from "@/features/workstreams/FileViewerTab";
import { SessionChat, SessionTabDot, SessionTabIcon } from "@/features/session/SessionChat";
import { SessionPresenceBadge } from "@/features/session/SessionPresenceBadge";
import { SpawnedByBadge, useAgentSpawnedSessionIds } from "@/features/session/SpawnedByBadge";
import { classifyArchivedStatus, restoreArchivedAgent } from "@/features/session/archivedAgents";
import { archiveStoppedSession, setSessionArchived } from "@/features/session/sessionArchive";
import { StopAgentDialog } from "@/features/session/StopAgentDialog";
import { DocPreviewTab } from "@/features/session/DocPreviewTab";
import { WebPreviewTab } from "@/features/session/WebPreviewTab";
import { TabTitle } from "@/features/session/TabTitle";
import { TerminalTab } from "@/features/session/TerminalTab";
import { SidebarTasksPanel } from "@/features/session/PlanTasksView";
import { PlanAside, PlanDocSync } from "@/features/plan/PlanAside";
import { ReviewWorkspace } from "@/features/review/ReviewWorkspace";
import { usePlanAside } from "@/features/plan/usePlanAside";
import { GoalAside } from "@/features/session/GoalAside";
import { useGoalAside } from "@/features/session/goalAsideState";
import { AgentPtyTab } from "@/features/multiplexer/AgentPtyTab";
import { EngineGlyph } from "@/components/EngineGlyph";
import { PtyTabDot } from "@/features/multiplexer/agentStateUi";
import {
  closeAgent,
  endAgent,
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
  useMultiplexer,
  type ResolvedEngine,
} from "@/features/multiplexer/engines";
import { ScriptDock } from "@/features/session/ScriptDock";
import { requestRun, setActiveWorkstream, useScriptsStore } from "@/features/session/scriptsStore";
import { openProjectSettings } from "@/features/projects/projectSettingsStore";
import { useTabDrag, type TabDragHandlers } from "@/features/session/useTabDrag";
import {
  agentStillWorking,
  draftWithQueued,
  dropSession,
  endSession,
  stopSession,
  transcriptToText,
  useSession,
  useSessionStore,
  type QueuedMessage,
} from "@/features/session/sessionStore";
import { isBusy } from "@/features/session/transcript";
import { useCommand } from "@/lib/keybindings/registry";
import { useChord } from "@/lib/keybindings/useChord";
import { wantsNativeMenu } from "@/lib/platform";
import { TAB_SELECT_COUNT, tabSelectId } from "@/lib/keybindings/commands";
import {
  importDroppedPaths,
  IMAGE_EXTENSIONS,
  type Attachment,
} from "@/features/session/attachments";
import { isFolderWorkstream, isGitProject } from "@/lib/domain/projects";
import { nextModeStage, stageLabel, type LaunchStage, type StageKey } from "@/lib/domain/stages";
import {
  activeShellIn,
  appendShell,
  archiveShellState,
  closePaneState,
  cycleActiveTab,
  defaultTabState,
  initialTabState,
  focusedPaneId,
  focusShell,
  isLastTab,
  mergePaneState,
  moveShellState,
  nextShellId,
  paneOf,
  paneShells,
  removeShellState,
  requestComposerFocus,
  seedWorkstreamTabs,
  useSeedWorkstreamTabs,
  selectLastTab,
  selectTabAt,
  setFocusedPane,
  splitPaneState,
  updateWorkstreamTabs,
  useWorkstreamTabsStore,
  type ArchivedStatus,
  type ShellTab,
  type WsTabState,
  isAgentTab,
  isSpawnedSessionTab,
} from "@/features/session/workstreamTabs";
import {
  leaves,
  minExtent,
  neighborPane,
  nodeAt,
  paneRects,
  setRatio,
  splitRects,
  type Direction,
  type Edge,
  type PaneId,
  type Rect,
  type SplitDir,
  type SplitNode,
  type SplitPath,
} from "@/features/session/paneTree";
import { WorkstreamDocsPanel } from "@/features/workstreams/WorkstreamDocsPanel";
import { chipLabel } from "@/features/workstreams/WorkstreamDocChips";
import { ProjectPipeline } from "@/features/board/ProjectPipeline";
import { LeadPanel } from "@/features/lead/LeadPanel";
import { useLeadDockExpanded, useProjectLead } from "@/features/lead/lead";
import { planGoalTarget, usePlanPath } from "@/features/board/pipelineData";
import { AttachPrDialog } from "@/features/workstreams/AttachPrDialog";
import {
  setNewWorkstreamStage,
  settleWorkstreamStage,
  useBootstrap,
  useDetachPr,
  useWorkstreams,
} from "@/lib/queries";
import { basename } from "@/lib/path";
import { shortSha } from "@/lib/format";
import { canSplitExtent, drawerWidth, splitPaneFloor } from "@/lib/layout";
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
  return !!(shell.docPreviewPath || shell.diffView || shell.filePath || shell.webView);
}

function isTerminalShell(shell: ShellTab): boolean {
  return !!shell.terminalView;
}

function isPtyShell(shell: ShellTab): boolean {
  return !!shell.ptyAgent;
}

function isArchivable(shell: ShellTab): boolean {
  return isAgentTab(shell);
}

function isAgentShell(shell: ShellTab): boolean {
  return !isViewerShell(shell) && !isTerminalShell(shell) && !isPtyShell(shell);
}


const FULL_RECT: Rect = { x: 0, y: 0, w: 1, h: 1 };

const pct = (fraction: number) => `${fraction * 100}%`;

// Panes render in creation order, not tree order, so a split never moves an existing pane's DOM node.
const byPaneIdSuffix = (a: PaneId, b: PaneId) => Number(a.slice(1)) - Number(b.slice(1));

const EDGE_SPLIT: Record<Edge, { dir: SplitDir; place: "before" | "after" }> = {
  left: { dir: "row", place: "before" },
  right: { dir: "row", place: "after" },
  top: { dir: "col", place: "before" },
  bottom: { dir: "col", place: "after" },
};

const EDGE_OVERLAY_CLASS: Record<Edge, string> = {
  left: "inset-y-0 left-0 w-1/2",
  right: "inset-y-0 right-0 w-1/2",
  top: "inset-x-0 top-0 h-1/2",
  bottom: "inset-x-0 bottom-0 h-1/2",
};

function classifyPtyArchivedStatus(state: AgentState | undefined): ArchivedStatus {
  return state === "done" ? "done" : "idle";
}

function TabCloseControl({
  shell,
  detaches,
  hoverOnly,
  onArchive,
  onClose,
}: {
  shell: ShellTab;
  detaches: boolean;
  hoverOnly: boolean;
  onArchive: () => void;
  onClose: () => void;
}) {
  const archivable = isArchivable(shell) && !detaches;
  const label = archivable ? `Archive ${shell.title}` : `Close ${shell.title}`;
  return (
    <button
      type="button"
      onClick={(e) => {
        e.stopPropagation();
        if (archivable) onArchive();
        else onClose();
      }}
      className={`inline-flex h-[18px] w-[18px] shrink-0 items-center justify-center rounded-control text-ink-dim hover:bg-edge/60 hover:text-ink ${
        hoverOnly ? "invisible group-hover:visible focus-visible:visible" : ""
      }`}
      title={label}
      aria-label={label}
    >
      <Icon name="close" size="sm" />
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
  onSplitDown: () => void;
  onClosePane: (() => void) | null;
  onFocusPane: ((dir: Direction) => void) | null;
  onNextTab: () => void;
  onPrevTab: () => void;
  onNewWorkstream: () => void;
  onToggleRun: (() => void) | null;
  onCycleStage: (() => void) | null;
  onToggleReview: (() => void) | null;
  onReturnQueued: (messages: QueuedMessage[]) => void;
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
  useCommand("pane.splitDown", props.onSplitDown);
  useCommand("pane.close", props.onClosePane);
  const focusPane = props.onFocusPane;
  useCommand("pane.focusLeft", focusPane ? () => focusPane("left") : null);
  useCommand("pane.focusRight", focusPane ? () => focusPane("right") : null);
  useCommand("pane.focusUp", focusPane ? () => focusPane("up") : null);
  useCommand("pane.focusDown", focusPane ? () => focusPane("down") : null);
  useCommand("tab.next", props.onNextTab);
  useCommand("tab.prev", props.onPrevTab);
  useCommand("ws.new", props.onNewWorkstream);
  useCommand("scripts.run", props.onToggleRun);
  useCommand("ws.cycleStage", props.onCycleStage);
  useCommand("ws.review", props.onToggleReview);
  useCommand("session.stop", running && sid ? () => void stopSession(sid, props.onReturnQueued) : null);
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

  const search = useSearch({ from: "/projects/$projectPath" });
  const navigate = useNavigate({ from: "/projects/$projectPath" });

  const selectedId = search.item;
  const lead = useProjectLead(projectPath);
  const leadExpanded = useLeadDockExpanded(projectPath);

  const tabsByWs = useWorkstreamTabsStore((s) => s.byWs);
  const views = (workstreams.data ?? []).filter((view) => view.item.status === "active");
  const leadMainId =
    selectedId === undefined && leadExpanded && lead?.view.sessionId ? lead.view.workstreamId : null;
  const shownId = selectedId ?? leadMainId;
  const activeView = shownId ? (views.find((v) => v.item.id === shownId) ?? null) : null;
  const wsId = activeView?.item.id ?? null;
  const multiplexer = useMultiplexer();
  const dockOpen = useScriptsStore((s) => (wsId ? !!s.dock[wsId]?.open : false));
  const dockPosition = useScriptsStore((s) => s.dockPosition);
  const [shellMenu, setShellMenu] = useState<ShellMenu | null>(null);
  const [newAgentMenu, setNewAgentMenu] = useState<{
    x: number;
    y: number;
    pane: PaneId;
  } | null>(null);
  const splitRightChord = useChord("tab.toggleSplit");
  const splitDownChord = useChord("pane.splitDown");
  const closePaneChord = useChord("pane.close");
  const [splitMenu, setSplitMenu] = useState<{ x: number; y: number; paneId: PaneId } | null>(
    null,
  );
  const [createOpen, setCreateOpen] = useState(false);
  const [pendingStage, setPendingStage] = useState<LaunchStage>();
  const [cardMenu, setCardMenu] = useState<{ x: number; y: number; view: WorkstreamView } | null>(
    null,
  );
  const cardPlanPath = usePlanPath(cardMenu?.view.item.id ?? null);
  const [attachPrTarget, setAttachPrTarget] = useState<WorkstreamView | null>(null);
  const detachPr = useDetachPr();
  const [renameShellId, setRenameShellId] = useState<number | null>(null);
  const [stopShellId, setStopShellId] = useState<number | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [docsModalShellId, setDocsModalShellId] = useState<number | null>(null);
  const { narrow } = useViewport();
  const { ref: paneRef, width: paneWidth } = useElementWidth<HTMLDivElement>();
  const narrowDrawer = useUiStore((state) => state.narrowDrawer);
  const diffHidden = useUiStore((state) => state.diffHidden);
  const diffWidth = useUiStore((state) => state.diffWidth);
  const setDiffWidth = useUiStore((state) => state.setDiffWidth);
  const [maximized, setMaximized] = useState<{ wsId: string; paneId: PaneId } | null>(null);
  const [dragRatio, setDragRatio] = useState<{
    path: SplitPath;
    node: SplitNode;
    ratio: number;
  } | null>(null);
  const splitRowRef = useRef<HTMLDivElement>(null);
  const dragHandlersRef = useRef<TabDragHandlers>({
    reorder: () => {},
    move: () => {},
    split: () => {},
    canEdgeSplit: () => false,
  });
  const { drag, dropTarget, suppressClickRef, startDrag } = useTabDrag(dragHandlersRef);

  useEffect(() => {
    if (search.create) {
      setCreateOpen(true);
      setPendingStage(search.stage);
      void navigate({
        search: (prev) => ({ ...prev, create: undefined, stage: undefined }),
        replace: true,
      });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [search.create]);

  const rightDock = !!activeView && dockOpen && dockPosition === "right";
  const showAside = narrow ? narrowDrawer === "diff" : !diffHidden;
  const asideWidth = narrow ? drawerWidth(diffWidth, paneWidth) : diffWidth;
  const showBoard = selectedId === undefined;
  // ProjectCommands registers ws.new only in the workspace branch.
  useCommand("ws.new", showBoard ? () => setCreateOpen(true) : null);

  useEffect(() => {
    if (selectedId === undefined) return;
    if (!workstreams.isSuccess || workstreams.isFetching) return;
    if (!views.some((v) => v.item.id === selectedId)) {
      void navigate({
        search: (prev) => ({
          ...prev,
          item: undefined,
          review: undefined,
          reviewFile: undefined,
          reviewLine: undefined,
        }),
        replace: true,
      });
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
      // An adopted PTY already runs on the server; its tab attaches when first shown.
      if (s.ptyAgent.adopted) continue;
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
          engine: base,
          projectPath,
          workstreamTitle: activeView.item.title,
          persistedTerminalId: pty.terminalId,
          notifyOnBlocked: multiplexer.notifyOnBlocked,
          session: { sessionId: pty.sessionId, resume, bypass },
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

  useSeedWorkstreamTabs(wsId, bootstrap.isSuccess, !!activeView?.item.parentWorkstreamId);

  useEffect(() => {
    if (!wsId) return;
    const tabs = tabsByWs[wsId] ?? defaultTabState();
    const focused = activeShellIn(tabs, focusedPaneId(tabs));
    if (focused && isAgentShell(focused)) requestComposerFocus(focused.id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wsId]);

  useEffect(() => {
    if (!wsId || !maximized || maximized.wsId !== wsId) return;
    const tabs = tabsByWs[wsId] ?? defaultTabState();
    if (focusedPaneId(tabs) !== maximized.paneId) setMaximized(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wsId, wsId ? tabsByWs[wsId]?.activePaneId : undefined]);

  const updateTabs = useCallback(
    (updater: (current: WsTabState) => WsTabState) => {
      if (!wsId) return;
      updateWorkstreamTabs((all) => {
        const current = all[wsId] ?? initialTabState();
        return { ...all, [wsId]: updater(current) };
      });
    },
    [wsId],
  );
  const openViewerTab = useCallback(
    (match: (shell: ShellTab) => boolean, title: string, fields: Partial<ShellTab>) => {
      updateTabs((current) => {
        const existing = current.shells.find(match);
        if (existing) return focusShell(current, existing.id);
        return appendShell(current, {
          id: nextShellId(current.shells, current.archived),
          title,
          mode: "general",
          draft: "",
          docs: [],
          attachments: [],
          paneId: focusedPaneId(current),
          ...fields,
        });
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

  const planTabs = wsId ? tabsByWs[wsId] : undefined;
  const asideSessionId = planTabs ? activeShellIn(planTabs, focusedPaneId(planTabs))?.sessionId : undefined;
  const planAside = usePlanAside(asideSessionId, narrow);
  const goalAside = useGoalAside(asideSessionId, wsId, planAside.available, narrow);
  const spawnedSessionIds = useAgentSpawnedSessionIds(wsId);

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
  const paneIds = tabs ? leaves(tabs.layout) : [];
  const focusedShell = tabs ? activeShellIn(tabs, focusedPaneId(tabs)) : null;
  // The Lead's tab shows the Lead panel where a workstream shows its git state: the Lead never changes files.
  const leadTab = !!lead?.view.sessionId && focusedShell?.sessionId === lead.view.sessionId;
  const zoomedPaneId =
    maximized && maximized.wsId === wsId && paneIds.length > 1 && paneIds.includes(maximized.paneId)
      ? maximized.paneId
      : null;
  const layout =
    tabs && dragRatio && nodeAt(tabs.layout, dragRatio.path) === dragRatio.node
      ? setRatio(tabs.layout, dragRatio.path, dragRatio.ratio)
      : tabs?.layout;
  const rects: Record<PaneId, Rect> = layout
    ? Object.fromEntries(paneRects(layout, 1, 1).map((p) => [p.paneId, p.rect]))
    : {};
  const splits = layout ? splitRects(layout, 1, 1) : [];
  const docsModalShell =
    docsModalShellId !== null ? (shells.find((s) => s.id === docsModalShellId) ?? null) : null;
  const contextShell = shellMenu ? shells.find((s) => s.id === shellMenu.shellId) : null;
  const archivedShells = tabs?.archived ?? [];
  const ptySessionIds = [...shells, ...archivedShells.map((a) => a.shell)]
    .map((s) => s.ptyAgent?.sessionId)
    .filter((id): id is string => !!id);
  const contextIsLast = !!tabs && !!contextShell && isLastTab(tabs, contextShell.id);
  const archiveBlockedReason = contextIsLast ? "Keep at least one agent in the strip." : null;
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
        | "webView"
        | "ptyAgent"
      >
    >,
  ) => modifyShell(shellId, (s) => ({ ...s, ...patch }));
  const setTerminalId = (shellId: number, terminalId: string) =>
    patchShell(shellId, { terminalView: { terminalId } });
  // A new terminal is one this desktop launched (Restart, a failed reattach), so the tab no longer adopts.
  const setPtyTerminalId = (shellId: number, engineId: string, terminalId: string) =>
    modifyShell(shellId, (s) => ({
      ...s,
      ptyAgent: {
        ...s.ptyAgent,
        engineId,
        terminalId,
        adopted: s.ptyAgent?.terminalId === terminalId ? s.ptyAgent.adopted : undefined,
      },
    }));
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
  const selectShell = (shellId: number) => updateTabs((current) => focusShell(current, shellId));
  const cycleTab = (dir: 1 | -1) => updateTabs((current) => cycleActiveTab(current, dir));
  const makeShell = (id: number, paneId: PaneId, ptyEngineId?: string): ShellTab => ({
    id,
    title: `Agent ${id}`,
    mode: "general",
    draft: "",
    docs: [],
    attachments: [],
    paneId,
    ...(ptyEngineId
      ? {
          ptyAgent: { engineId: ptyEngineId, started: false },
        }
      : {}),
  });
  const appendToPane = (paneId: PaneId, build: (id: number, target: PaneId) => ShellTab) =>
    updateTabs((current) => {
      const id = nextShellId(current.shells, current.archived);
      const target = leaves(current.layout).includes(paneId) ? paneId : focusedPaneId(current);
      return appendShell(current, build(id, target));
    });
  const addShellToPane = (paneId: PaneId, ptyEngineId?: string) =>
    appendToPane(paneId, (id, target) => makeShell(id, target, ptyEngineId));
  const addWebPreviewToPane = (paneId: PaneId) =>
    appendToPane(paneId, (id, target) => ({
      ...makeShell(id, target),
      title: "Web preview",
      webView: { url: "" },
    }));
  const navigateWebPreview = (shell: ShellTab, url: string) =>
    patchShell(shell.id, {
      webView: { url },
      ...(shell.titleLocked ? {} : { title: new URL(url).host }),
    });
  const addShell = (ptyEngineId?: string) => {
    updateTabs((current) => {
      const id = nextShellId(current.shells, current.archived);
      return appendShell(current, makeShell(id, focusedPaneId(current), ptyEngineId));
    });
  };
  const newTabEngineId = () => (multiplexer.enabled ? defaultEngine(multiplexer)?.id : undefined);
  const onNewTabDefault = () => addShell(newTabEngineId());
  const openNewAgentMenu = (e: ReactMouseEvent, pane: PaneId) => {
    e.stopPropagation();
    const r = e.currentTarget.getBoundingClientRect();
    setNewAgentMenu({ x: r.left, y: r.bottom + 4, pane });
  };
  const openSplitMenu = (e: ReactMouseEvent, paneId: PaneId) => {
    e.stopPropagation();
    const r = e.currentTarget.getBoundingClientRect();
    setSplitMenu({ x: r.left, y: r.bottom + 4, paneId });
  };
  const onPlus = (e: ReactMouseEvent, paneId: PaneId) => {
    if (!multiplexer.enabled) return addShellToPane(paneId);
    openNewAgentMenu(e, paneId);
  };
  const ptyEngineFor = (engineId: string): ResolvedEngine => engineOrFallback(multiplexer, engineId);
  const isLivePty = (shell: ShellTab): boolean => multiplexer.enabled && !!shell.ptyAgent;
  const canSplit = (paneId: PaneId, dir: SplitDir): boolean => {
    const box = splitRowRef.current?.getBoundingClientRect();
    const rect = rects[paneId];
    if (!box || !rect) return false;
    return canSplitExtent(dir === "row" ? rect.w * box.width : rect.h * box.height);
  };
  const canEdgeSplit = (id: number, paneId: PaneId, edge: Edge): boolean => {
    if (!tabs) return false;
    const shell = tabs.shells.find((s) => s.id === id);
    if (shell && paneOf(tabs, shell) === paneId && paneShells(tabs, paneId).length === 1) {
      return false;
    }
    return canSplit(paneId, EDGE_SPLIT[edge].dir);
  };
  const splitAtEdge = (id: number, paneId: PaneId, edge: Edge) => {
    const { dir, place } = EDGE_SPLIT[edge];
    updateTabs((current) => splitPaneState(current, paneId, dir, place, id));
    setMaximized(null);
  };
  const splitPane = (paneId: PaneId, dir: SplitDir) => {
    if (!canSplit(paneId, dir)) return;
    updateTabs((current) => {
      const id = nextShellId(current.shells, current.archived);
      const withShell = { ...current, shells: [...current.shells, makeShell(id, paneId, newTabEngineId())] };
      return splitPaneState(withShell, paneId, dir, "after", id);
    });
    setMaximized(null);
  };
  const splitFocused = (dir: SplitDir) => {
    if (tabs) splitPane(focusedPaneId(tabs), dir);
  };
  const focusPane = (dir: Direction) => {
    if (!tabs) return;
    const target = neighborPane(tabs.layout, focusedPaneId(tabs), dir);
    if (!target) return;
    updateTabs((current) => setFocusedPane(current, target));
    const shell = activeShellIn(tabs, target);
    if (shell && isAgentShell(shell)) {
      requestComposerFocus(shell.id);
      return;
    }
    const inPane = `[data-pane="${target}"] [data-pane-content] `;
    const focusable = document.querySelector<HTMLElement>(
      `${inPane}textarea, ${inPane}[contenteditable="true"]`,
    );
    if (focusable) focusable.focus();
    else if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
  };
  const mergePane = (paneId: PaneId) => {
    updateTabs((current) => mergePaneState(current, paneId));
    setMaximized(null);
  };
  const detaches = (shell: ShellTab) =>
    !!shell.ptyAgent?.adopted || (!!tabs && isSpawnedSessionTab(tabs, shell, spawnedSessionIds));
  // Unlike a composer tab, an agent-spawned session runs for every desktop; only its parent or an archive ends it.
  const endOrDetach = (shell: ShellTab) => {
    if (shell.sessionId && !detaches(shell)) void endSession(shell.sessionId);
  };
  const closePane = (paneId: PaneId) => {
    if (!tabs || paneIds.length < 2) return;
    setMaximized(null);
    const closing = paneShells(tabs, paneId);
    for (const s of closing) {
      endOrDetach(s);
      if (s.terminalView?.terminalId) void commands.terminalClose(s.terminalView.terminalId);
      if (s.ptyAgent && wsId) closeAgent(wsId, s.id, s.ptyAgent.adopted);
    }
    updateTabs((current) => closePaneState(current, paneId));
    for (const s of closing) if (s.sessionId) dropSession(s.sessionId);
  };
  const moveShellToPane = (shellId: number, toPaneId: PaneId) =>
    updateTabs((current) => moveShellState(current, shellId, toPaneId));
  const reorderShellInPane = (shellId: number, paneId: PaneId, toIndex: number) => {
    updateTabs((current) => {
      const inPane = (s: ShellTab) => paneOf(current, s) === paneId;
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
  dragHandlersRef.current = {
    reorder: reorderShellInPane,
    move: moveShellToPane,
    split: splitAtEdge,
    canEdgeSplit,
  };
  const closeShell = (shellId: number) => {
    if (!tabs) return;
    const shell = tabs.shells.find((s) => s.id === shellId);
    if (!shell) return;
    if (isLastTab(tabs, shellId)) return;
    endOrDetach(shell);
    if (shell.terminalView?.terminalId) void commands.terminalClose(shell.terminalView.terminalId);
    if (shell.ptyAgent && wsId) closeAgent(wsId, shellId, shell.ptyAgent.adopted);
    updateTabs((current) => removeShellState(current, shellId));
    if (shell.sessionId) dropSession(shell.sessionId);
  };
  const archiveShellNow = (shellId: number, stopping = false) => {
    const shell = shells.find((s) => s.id === shellId);
    if (!shell) return;
    let status: ArchivedStatus;
    if (shell.ptyAgent) {
      const ptyState = wsId
        ? useMultiplexerStore.getState().agents[agentKey(wsId, shellId)]?.state
        : undefined;
      status = classifyPtyArchivedStatus(ptyState);
      if (wsId) endAgent(wsId, shellId, shell.ptyAgent.terminalId);
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
      const sessionId = shell.sessionId;
      if (stopping && wsId) {
        void endSession(sessionId).then(() => archiveStoppedSession(wsId, sessionId));
      } else {
        void endSession(sessionId);
        if (wsId) void setSessionArchived(wsId, sessionId, true);
      }
    }
  };
  const archiveShell = (shellId: number) => {
    const shell = shells.find((s) => s.id === shellId);
    const session = shell && isAgentShell(shell) && shell.sessionId
      ? useSessionStore.getState().sessions[shell.sessionId]
      : undefined;
    if (session && agentStillWorking(session)) setStopShellId(shellId);
    else archiveShellNow(shellId);
  };
  const archiveOrCloseShell = (shellId: number) => {
    const shell = shells.find((s) => s.id === shellId);
    if (!shell) return;
    if (!isArchivable(shell) || detaches(shell)) {
      closeShell(shellId);
      return;
    }
    archiveShell(shellId);
  };
  const reopenLastClosedShell = () => {
    const newest = tabs?.archived?.[0];
    if (newest && wsId) restoreArchivedAgent(wsId, newest.shell.id);
  };
  const openDoc = (id: string) => {
    void navigate({ to: "/library", search: { doc: id, from: projectPath } });
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
      search: (prev) => ({
        ...prev,
        item: id,
        review: undefined,
        reviewFile: undefined,
        reviewLine: undefined,
      }),
      replace: true,
    });
  };
  // The dialog calls onClose() before onCreated(); this closure keeps the render's pendingStage, so a ref would lose it.
  const onCreated = (view: WorkstreamView, created?: boolean) => {
    if (created) seedWorkstreamTabs(view.item.id, pendingStage);
    if (created && pendingStage) void setNewWorkstreamStage(view.item.id, pendingStage);
    selectItem(view.item.id);
  };
  const closeCreate = () => {
    setCreateOpen(false);
    setPendingStage(undefined);
  };
  const reviewable = !!activeView && !isFolderWorkstream(activeView);
  const reviewing = reviewable && !!search.review;
  const openReview = () => {
    void navigate({
      search: (prev) => ({ ...prev, review: "files", reviewFile: undefined, reviewLine: undefined }),
    });
  };
  const toggleReview = () => {
    if (!reviewing) return openReview();
    void navigate({
      search: (prev) => ({ ...prev, review: undefined, reviewFile: undefined, reviewLine: undefined }),
    });
  };
  const changeShellMode = (shell: ShellTab, key: StageKey) => {
    if (!activeView || key === shell.mode) return;
    patchShell(shell.id, { mode: key });
    settleWorkstreamStage(activeView.item.id, key);
  };
  const addAttachmentsToShell = (shellId: number, atts: Attachment[]) => {
    if (atts.length === 0) return;
    modifyShell(shellId, (s) => {
      const existing = s.attachments;
      const have = new Set(existing.map((a) => a.path));
      const added = atts.filter((a) => !have.has(a.path));
      return added.length ? { ...s, attachments: [...existing, ...added] } : s;
    });
  };
  const returnQueuedToShell = (shellId: number, messages: QueuedMessage[]) => {
    modifyShell(shellId, (s) => ({ ...s, draft: draftWithQueued(s.draft, messages) }));
    addAttachmentsToShell(shellId, messages.flatMap((m) => m.attachments));
  };
  const removeAttachmentFromShell = (shellId: number, path: string) =>
    modifyShell(shellId, (s) => ({
      ...s,
      attachments: s.attachments.filter((a) => a.path !== path),
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
      const staged = s.attachments.some((a) => a.path === path);
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
    focused: shell.id === focusedShell?.id,
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
    onReturnQueued: (messages: QueuedMessage[]) => returnQueuedToShell(shell.id, messages),
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
  const startSplitResize = (
    split: { path: SplitPath; node: SplitNode; rect: Rect },
    event: ReactPointerEvent<HTMLDivElement>,
  ) => {
    const box = splitRowRef.current?.getBoundingClientRect();
    if (!box) return;
    const row = split.node.dir === "row";
    const axis: "x" | "y" = row ? "x" : "y";
    const size = row ? split.rect.w * box.width : split.rect.h * box.height;
    const floorA = splitPaneFloor(size, minExtent(split.node.a, axis), minExtent(split.node.b, axis));
    const floorB = splitPaneFloor(size, minExtent(split.node.b, axis), minExtent(split.node.a, axis));
    const startPx = split.node.ratio * size;
    let last = split.node.ratio;
    startEdgeDrag(event, {
      startWidth: startPx,
      min: Math.min(floorA, startPx),
      max: Math.max(Math.max(floorA, size - floorB), startPx),
      direction: 1,
      axis,
      onResize: (px) => {
        last = clamp(px / size, 0.2, 0.8);
        setDragRatio({ path: split.path, node: split.node, ratio: last });
      },
      onEnd: () => {
        updateTabs((current) =>
          nodeAt(current.layout, split.path) === split.node
            ? { ...current, layout: setRatio(current.layout, split.path, last) }
            : current,
        );
        setDragRatio(null);
      },
    });
  };
  const startDiffResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    startEdgeDrag(event, {
      startWidth: diffWidth,
      min: DIFF_SIDEBAR_MIN,
      max: DIFF_SIDEBAR_MAX,
      direction: -1,
      onResize: setDiffWidth,
    });
  };

  const renderTabPill = (
    shell: ShellTab,
    opts: {
      active: boolean;
      paneFocused: boolean;
      pane: PaneId;
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
      className={`group relative inline-flex h-7 shrink-0 cursor-pointer select-none items-center gap-1.5 rounded-control border pl-2.5 text-ui ${
        opts.closable ? "pr-1" : "pr-2.5"
      } ${
        !opts.active
          ? "border-transparent bg-panel-2 font-medium text-ink-dim hover:text-ink"
          : opts.paneFocused
            ? "border-edge bg-panel font-semibold text-ink shadow-sm"
            : "border-edge bg-panel font-medium text-ink-2"
      } ${drag?.id === shell.id ? "opacity-60 ring-2 ring-inset ring-accent" : ""}`}
    >
      <span className="flex min-w-0 items-center gap-1.5">
        {shell.diffView ? (
          <span className="text-ink-dim" title="Diff view">
            <Icon name="diff" size="sm" />
          </span>
        ) : isTerminalShell(shell) ? (
          <span className="text-ink-dim">
            <Icon name="terminal" size="sm" />
          </span>
        ) : shell.webView ? (
          <span className="text-ink-dim">
            <Icon name="globe" size="sm" />
          </span>
        ) : isLivePty(shell) && shell.ptyAgent ? (
          (() => {
            const e = ptyEngineFor(shell.ptyAgent.engineId);
            return (
              <EngineGlyph
                glyph={e.glyph}
                accentToken={e.accentToken}
                className="text-ui-sm leading-none"
              />
            );
          })()
        ) : isViewerShell(shell) ? (
          <span className="text-ink-dim">
            <Icon name="doc" size="sm" />
          </span>
        ) : (
          <SessionTabIcon mode={shell.mode} />
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
          detaches={detaches(shell)}
          hoverOnly={!opts.active}
          onArchive={() => archiveShell(shell.id)}
          onClose={() => closeShell(shell.id)}
        />
      )}
    </div>
  );

  const renderLeafStrip = (paneId: PaneId) => {
    if (!tabs) return null;
    const first = paneId === paneIds[0];
    const multiPane = paneIds.length > 1;
    const active = activeShellIn(tabs, paneId);
    const paneFocused = !multiPane || focusedPaneId(tabs) === paneId;
    return (
      <div
        data-glass
        data-pane={paneId}
        className={`relative z-10 flex h-10 items-center justify-between gap-3 border-b border-edge ${GLASS_BAND} ${
          first ? "pl-5 pr-2.5" : "px-3"
        } ${dropTarget?.paneId === paneId && dropTarget.edge === null ? "bg-accent/10" : ""}`}
      >
        <ScrollableTabStrip
          activeId={active?.id ?? null}
          trailing={
            <>
              <IconButton
                onClick={(e) => onPlus(e, paneId)}
                title="New agent"
                aria-label="New agent"
              >
                <Icon name="add" />
              </IconButton>
              <IconButton
                onClick={() => addWebPreviewToPane(paneId)}
                title="New web preview"
                aria-label="New web preview"
              >
                <Icon name="globe" />
              </IconButton>
            </>
          }
        >
          {paneShells(tabs, paneId).map((shell) =>
            renderTabPill(shell, {
              active: shell.id === active?.id,
              paneFocused,
              pane: paneId,
              onSelect: () => selectShell(shell.id),
              closable: !isLastTab(tabs, shell.id),
            }),
          )}
        </ScrollableTabStrip>
        <div className="flex shrink-0 items-center gap-1">
          <SpawnedByBadge workstreamId={wsId ?? ""} sessionId={active?.sessionId} />
          <SessionPresenceBadge sessionId={active?.sessionId} />
          <IconButton
            onClick={(e) => openSplitMenu(e, paneId)}
            title="Split pane"
            aria-label="Split pane"
          >
            <Icon name="layoutSplit2" />
          </IconButton>
          {multiPane && (
            <IconButton
              onClick={() => closePane(paneId)}
              title="Close pane and end its agents"
              aria-label="Close pane and end its agents"
            >
              <Icon name="close" size="sm" />
            </IconButton>
          )}
        </div>
      </div>
    );
  };

  const renderPaneContent = (shell: ShellTab | null) => {
    if (!activeView || !shell) {
      return (
        <div className="flex min-h-0 flex-1 items-center justify-center p-6 text-body text-ink-dim">
          No agent in this pane
        </div>
      );
    }
    if (shell.docPreviewPath) {
      return (
        <DocPreviewTab
          key={`${activeView.item.id}:${shell.id}`}
          workstreamId={activeView.item.id}
          path={shell.docPreviewPath}
          title={shell.title}
        />
      );
    }
    if (shell.webView) {
      return (
        <WebPreviewTab
          key={`${activeView.item.id}:${shell.id}`}
          url={shell.webView.url}
          onNavigate={(url) => navigateWebPreview(shell, url)}
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
          adopted={!!shell.ptyAgent.adopted}
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
    const leadShell = leadMainId ? shells.find((s) => s.sessionId === lead?.view.sessionId) : undefined;
    return (
      <>
        <ProjectPipeline
          projectPath={projectPath}
          projectName={project.mappedName}
          views={views}
          folder={folderProject}
          onSelect={selectItem}
          onNew={() => setCreateOpen(true)}
          onCardContextMenu={
            folderProject ? undefined : (view, x, y) => setCardMenu({ x, y, view })
          }
          leadChat={leadShell ? renderPaneContent(leadShell) : null}
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
            {cardPlanPath && (
              <ContextMenuItem
                onClick={() => {
                  const workstreamId = cardMenu.view.item.id;
                  openGoalSheet({
                    mode: "start",
                    workstreamId,
                    source: "plan",
                    planPath: cardPlanPath,
                    target: planGoalTarget(workstreamId, cardPlanPath),
                  });
                  setCardMenu(null);
                }}
              >
                Run until met…
              </ContextMenuItem>
            )}
          </ContextMenu>
        )}
        <AttachPrDialog target={attachPrTarget} onClose={() => setAttachPrTarget(null)} />
        <CreateWorkstreamDialog
          projectPath={projectPath}
          folder={folderProject}
          open={createOpen}
          onClose={closeCreate}
          onCreated={onCreated}
        />
      </>
    );
  }

  if (selectedId && !activeView && workstreams.isFetching) return <PageSpinner />;

  return (
    <div ref={paneRef} className="relative flex h-full min-h-0 bg-surface">
      <ProjectCommands
        sessionId={focusedShell?.sessionId}
        onReturnQueued={(messages) => focusedShell && returnQueuedToShell(focusedShell.id, messages)}
        onNewTab={onNewTabDefault}
        onArchiveTab={focusedShell ? () => archiveOrCloseShell(focusedShell.id) : null}
        onCloseTab={focusedShell ? () => closeShell(focusedShell.id) : null}
        onReopenTab={tabs?.archived?.length ? reopenLastClosedShell : null}
        onSelectTab={(index) => updateTabs((current) => selectTabAt(current, index))}
        onSelectLastTab={() => updateTabs(selectLastTab)}
        onToggleSplit={() => splitFocused("row")}
        onSplitDown={() => splitFocused("col")}
        onClosePane={tabs && paneIds.length > 1 ? () => mergePane(focusedPaneId(tabs)) : null}
        onFocusPane={paneIds.length > 1 && zoomedPaneId === null ? focusPane : null}
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
        onToggleReview={reviewable ? toggleReview : null}
      />
      {reviewing && activeView ? (
        <ReviewWorkspace
          key={activeView.item.id}
          view={activeView}
          tab={search.review!}
          file={search.reviewFile}
          line={search.reviewLine}
          onOpenDoc={openDoc}
        />
      ) : (
        <>
          <section className="flex min-w-0 flex-1 flex-col">
            <div className="flex min-h-0 flex-1">
              <main className="flex min-w-0 flex-1 flex-col">
                {shellMenu &&
                  contextShell &&
                  tabs &&
                  (() => {
                    const pane = paneOf(tabs, contextShell);
                    const onlyTab = paneShells(tabs, pane).length === 1;
                    const canRow = canSplit(pane, "row");
                    const canCol = canSplit(pane, "col");
                    const nextPane = paneIds[(paneIds.indexOf(pane) + 1) % paneIds.length];
                    return (
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
                        <ContextMenuItem
                          disabled={onlyTab || !canRow}
                          hint={!canRow ? "Not enough room" : undefined}
                          onClick={() => {
                            updateTabs((current) =>
                              splitPaneState(current, paneOf(current, contextShell), "row", "after", contextShell.id),
                            );
                            setShellMenu(null);
                          }}
                        >
                          Split right with this tab
                        </ContextMenuItem>
                        <ContextMenuItem
                          disabled={onlyTab || !canCol}
                          hint={!canCol ? "Not enough room" : undefined}
                          onClick={() => {
                            updateTabs((current) =>
                              splitPaneState(current, paneOf(current, contextShell), "col", "after", contextShell.id),
                            );
                            setShellMenu(null);
                          }}
                        >
                          Split down with this tab
                        </ContextMenuItem>
                        {paneIds.length > 1 && (
                          <ContextMenuItem
                            onClick={() => {
                              moveShellToPane(contextShell.id, nextPane);
                              setShellMenu(null);
                            }}
                          >
                            Move to next pane
                          </ContextMenuItem>
                        )}
                        {paneIds.length > 1 &&
                          (zoomedPaneId === pane ? (
                            <ContextMenuItem
                              onClick={() => {
                                setMaximized(null);
                                setShellMenu(null);
                              }}
                            >
                              Restore split view
                            </ContextMenuItem>
                          ) : (
                            <ContextMenuItem
                              onClick={() => {
                                selectShell(contextShell.id);
                                if (wsId) setMaximized({ wsId, paneId: pane });
                                setShellMenu(null);
                              }}
                            >
                              Expand pane
                            </ContextMenuItem>
                          ))}
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
                          danger={isArchivable(contextShell) && !detaches(contextShell)}
                          disabled={contextIsLast}
                          onClick={() => {
                            closeShell(contextShell.id);
                            setShellMenu(null);
                          }}
                        >
                          {isArchivable(contextShell) && !detaches(contextShell) ? "Close agent" : "Close tab"}
                        </ContextMenuItem>
                      </ContextMenu>
                    );
                  })()}
                {splitMenu &&
                  (() => {
                    const menu = splitMenu;
                    const canRow = canSplit(menu.paneId, "row");
                    const canCol = canSplit(menu.paneId, "col");
                    return (
                      <ContextMenu position={menu} onClose={() => setSplitMenu(null)}>
                        <ContextMenuItem
                          icon="layoutSplit2"
                          chord={splitRightChord}
                          disabled={!canRow}
                          hint={!canRow ? "Not enough room" : undefined}
                          onClick={() => {
                            splitPane(menu.paneId, "row");
                            setSplitMenu(null);
                          }}
                        >
                          Split right
                        </ContextMenuItem>
                        <ContextMenuItem
                          icon="layoutSplitDown"
                          chord={splitDownChord}
                          disabled={!canCol}
                          hint={!canCol ? "Not enough room" : undefined}
                          onClick={() => {
                            splitPane(menu.paneId, "col");
                            setSplitMenu(null);
                          }}
                        >
                          Split down
                        </ContextMenuItem>
                        {paneIds.length > 1 &&
                          (zoomedPaneId === menu.paneId ? (
                            <ContextMenuItem
                              icon="paneRestore"
                              onClick={() => {
                                setMaximized(null);
                                setSplitMenu(null);
                              }}
                            >
                              Restore split view
                            </ContextMenuItem>
                          ) : (
                            <ContextMenuItem
                              icon="paneZoom"
                              onClick={() => {
                                if (wsId) setMaximized({ wsId, paneId: menu.paneId });
                                setSplitMenu(null);
                              }}
                            >
                              Expand pane
                            </ContextMenuItem>
                          ))}
                        <MenuSeparator />
                        <ContextMenuItem
                          icon="close"
                          chord={closePaneChord}
                          disabled={paneIds.length === 1}
                          onClick={() => {
                            mergePane(menu.paneId);
                            setSplitMenu(null);
                          }}
                        >
                          Close pane (keep tabs)
                        </ContextMenuItem>
                      </ContextMenu>
                    );
                  })()}
                {newAgentMenu &&
                  (() => {
                    const menu = newAgentMenu;
                    const def = defaultEngine(multiplexer);
                    const spawn = (engineId?: string) => addShellToPane(menu.pane, engineId);
                    return (
                      <ContextMenu position={menu} onClose={() => setNewAgentMenu(null)}>
                        <div className="px-2 pb-1 pt-0.5 text-ui text-ink-dim">
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
                                className="text-body leading-none"
                              />
                              <span className="min-w-0 flex-1 truncate">{engine.label}</span>
                              {engine.id === def?.id && (
                                <span className="text-meta text-ink-dim">⌘T</span>
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
                <StopAgentDialog
                  title={shells.find((s) => s.id === stopShellId)?.title ?? null}
                  onClose={() => setStopShellId(null)}
                  onConfirm={() => {
                    if (stopShellId !== null) archiveShellNow(stopShellId, true);
                    setStopShellId(null);
                  }}
                />
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

                {activeView && tabs ? (
                  <div ref={splitRowRef} className="relative min-h-0 flex-1">
                    {[...paneIds].sort(byPaneIdSuffix).map((paneId) => {
                      const zoomedAway = zoomedPaneId !== null && zoomedPaneId !== paneId;
                      const r = zoomedPaneId === paneId ? FULL_RECT : rects[paneId];
                      return (
                        <div
                          key={paneId}
                          data-pane={paneId}
                          className={`absolute flex min-h-0 min-w-0 flex-col overflow-hidden ${zoomedAway ? "hidden" : ""} ${
                            r.x > 1e-6 ? "border-l border-edge" : ""
                          } ${r.y > 1e-6 ? "border-t border-edge" : ""}`}
                          style={{ left: pct(r.x), top: pct(r.y), width: pct(r.w), height: pct(r.h) }}
                          onPointerDownCapture={() => {
                            if (paneIds.length > 1 && focusedPaneId(tabs) !== paneId) {
                              updateTabs((current) => setFocusedPane(current, paneId));
                            }
                          }}
                        >
                          {renderLeafStrip(paneId)}
                          <div
                            data-pane-content
                            onContextMenu={(e) => {
                              if (e.defaultPrevented || wantsNativeMenu(e.target)) return;
                              e.preventDefault();
                              setSplitMenu({ x: e.clientX, y: e.clientY, paneId });
                            }}
                            className={`relative flex min-h-0 flex-1 flex-col ${
                              dropTarget?.paneId === paneId && dropTarget.edge === null
                                ? "ring-2 ring-inset ring-accent/60"
                                : ""
                            }`}
                          >
                            {renderPaneContent(activeShellIn(tabs, paneId))}
                            {dropTarget?.paneId === paneId && dropTarget.edge && (
                              <div
                                className={`pointer-events-none absolute bg-accent/15 ring-2 ring-inset ring-accent/60 ${
                                  EDGE_OVERLAY_CLASS[dropTarget.edge]
                                }`}
                              />
                            )}
                          </div>
                        </div>
                      );
                    })}
                    {zoomedPaneId === null &&
                      splits.map((split) => {
                        const { rect, node } = split;
                        const row = node.dir === "row";
                        return (
                          <div
                            key={split.path.join("") || "root"}
                            role="separator"
                            aria-orientation={row ? "vertical" : "horizontal"}
                            aria-label="Resize agent split"
                            className={`absolute z-10 bg-transparent hover:bg-edge/60 ${
                              row
                                ? "w-2 -translate-x-1/2 cursor-col-resize"
                                : "h-2 -translate-y-1/2 cursor-row-resize"
                            }`}
                            style={
                              row
                                ? {
                                    left: pct(rect.x + rect.w * node.ratio),
                                    top: pct(rect.y),
                                    height: pct(rect.h),
                                  }
                                : {
                                    left: pct(rect.x),
                                    top: pct(rect.y + rect.h * node.ratio),
                                    width: pct(rect.w),
                                  }
                            }
                            onPointerDown={(e) => startSplitResize(split, e)}
                          />
                        );
                      })}
                  </div>
                ) : (
                  <div className="min-h-0 flex-1 overflow-y-auto p-5">
                    <div className="mx-auto max-w-3xl space-y-4">
                      <div className="rounded-lg border border-edge bg-panel p-6 text-center">
                        <p className="text-title text-ink">Create your first workstream</p>
                        <p className="mx-auto mt-1 max-w-md text-body leading-prose text-ink-dim">
                          {folderProject
                            ? "Each workstream is its own conversation, all running in this folder. Start as many as you need."
                            : "Each workstream gets its own branch and worktree, so agents can research, plan, and implement without touching your checkout."}
                        </p>
                        <Button variant="primary" className="mt-4" onClick={() => setCreateOpen(true)}>
                          <Icon name="add" />
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
              className="relative z-10 -mx-1 w-2 shrink-0 cursor-col-resize bg-transparent hover:bg-edge/60"
              onPointerDown={startDiffResize}
            />
          )}
          <aside
            className={`shrink-0 overflow-hidden border-l border-edge bg-panel ${
              narrow ? "absolute inset-y-0 right-0 z-30 shadow-lg" : "relative"
            }`}
            style={{ width: showAside ? asideWidth : 0 }}
            inert={!showAside}
          >
            <div
              className="absolute inset-y-0 right-0 flex flex-col"
              style={{ width: asideWidth }}
            >
              {(planAside.available || goalAside.available) && (
                <div className="flex h-9 shrink-0 items-center border-b border-edge px-2">
                  <SegmentedControl
                    aria-label="Right panel"
                    value={goalAside.view}
                    options={[
                      ...(planAside.available ? [{ value: "plan" as const, label: "Plan" }] : []),
                      ...(goalAside.available ? [{ value: "goal" as const, label: "Goal" }] : []),
                      { value: "changes" as const, label: leadTab ? "Lead" : "Changes" },
                    ]}
                    onChange={goalAside.setView}
                  />
                </div>
              )}
              {goalAside.view === "goal" && activeView ? (
                <div className="min-h-0 flex-1">
                  <GoalAside sessionId={focusedShell?.sessionId} workstreamId={activeView.item.id} />
                </div>
              ) : goalAside.view === "plan" && activeView && focusedShell ? (
                <div className="min-h-0 flex-1">
                  <PlanAside
                    view={activeView}
                    shell={focusedShell}
                    onOpenDoc={openDoc}
                    onOpenFile={(path) => openDocPreview(path, basename(path).replace(/\.md$/, ""))}
                    onReviewChanges={openReview}
                  />
                </div>
              ) : leadTab ? (
                <>
                  <LeadPanel projectPath={projectPath} views={views} className="min-h-0 flex-1" />
                  <SidebarTasksPanel sessionId={focusedShell?.sessionId} />
                </>
              ) : (
                <>
                  {!folderProject && <GitPanel view={activeView ?? null} />}
                  <div className="flex min-h-0 flex-1 flex-col">
                    <ChangesPanel
                      workstreamId={activeView?.item.id ?? null}
                      onOpenDoc={openDocPreview}
                      onOpenDiff={openDiff}
                      onOpenFile={openFile}
                      onOpenReview={reviewable ? openReview : undefined}
                      filesOnly={folderProject}
                      baseBranch={activeView?.item.baseBranch ?? null}
                      aheadOfBase={activeView?.git.base.ahead ?? null}
                    />
                  </div>
                  <SidebarTasksPanel sessionId={focusedShell?.sessionId} />
                </>
              )}
              {rightDock && showAside && (
                <ScriptDock
                  key={`${activeView!.item.id}:right`}
                  workstreamId={activeView!.item.id}
                  projectPath={projectPath}
                  onEditScripts={() => openProjectSettings(projectPath)}
                  orientation="right"
                />
              )}
              {!folderProject && !leadTab && <PrChecksPanel view={activeView ?? null} />}
            </div>
          </aside>
        </>
      )}

      {tabs?.shells.map((s) =>
        s.mode === "implement" && s.sessionId ? (
          <PlanDocSync key={s.sessionId} sessionId={s.sessionId} />
        ) : null,
      )}

      <CreateWorkstreamDialog
        projectPath={projectPath}
        folder={folderProject}
        open={createOpen}
        onClose={closeCreate}
        onCreated={onCreated}
      />

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
          className="pointer-events-none fixed z-50 scale-105 rounded-md border border-accent/70 bg-panel-2 px-2.5 py-1.5 text-ui font-medium text-ink shadow-2xl ring-2 ring-accent/30"
          style={{ left: drag.x + 12, top: drag.y + 12 }}
        >
          {drag.title}
        </div>
      )}
    </div>
  );
}
