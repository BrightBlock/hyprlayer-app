import { useNavigate, useParams, useSearch } from "@tanstack/react-router";
import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";

import {
  Button,
  ContextMenu,
  ContextMenuItem,
  Dialog,
  IconButton,
  PageError,
  PageSpinner,
  TextInput,
} from "../components/ui";
import { open } from "@tauri-apps/plugin-dialog";

import { ArchiveIcon, DiffIcon, DocIcon, PlusIcon, SplitIcon, TerminalIcon, XIcon } from "../components/icons";
import { ScrollableTabStrip } from "../components/ScrollableTabs";
import { commands, type WorkstreamView } from "../lib/bindings";
import { CreateWorkstreamDialog } from "../features/workstreams/CreateWorkstreamDialog";
import { ChangesPanel } from "../features/workstreams/ChangesPanel";
import { GitPanel } from "../features/workstreams/GitPanel";
import { PrChecksPanel } from "../features/workstreams/PrChecksPanel";
import { DiffTab } from "../features/workstreams/DiffTab";
import { FileViewerTab } from "../features/workstreams/FileViewerTab";
import { SessionChat, SessionTabDot, SessionTabIcon } from "../features/session/SessionChat";
import { ArchivedAgentsView } from "../features/session/ArchivedAgentsView";
import { DocPreviewTab } from "../features/session/DocPreviewTab";
import { TabTitle } from "../features/session/TabTitle";
import { TerminalTab } from "../features/session/TerminalTab";
import { ScriptDock } from "../features/session/ScriptDock";
import { requestRun, setActiveWorkstream, useScriptsStore } from "../features/session/scriptsStore";
import { openProjectSettings } from "../features/projects/projectSettingsStore";
import { useTabDrag, type TabDragHandlers } from "../features/session/useTabDrag";
import {
  killSession,
  stopSession,
  transcriptToText,
  useSession,
  useSessionStore,
} from "../features/session/sessionStore";
import { isBusy, isTerminal, type SessionState } from "../features/session/transcript";
import { useCommand } from "../lib/keybindings/registry";
import {
  importDroppedPaths,
  IMAGE_EXTENSIONS,
  type Attachment,
} from "../features/session/attachments";
import { nextModeStage, stageLabel, type StageKey } from "../features/session/stages";
import {
  archiveShellState,
  clearArchiveOnRefocus,
  cycleActiveTab,
  defaultTabState,
  nextShellId,
  removeShellState,
  requestComposerFocus,
  unarchiveShellState,
  updateWorkstreamTabs,
  useWorkstreamTabsStore,
  type ArchivedStatus,
  type WsTabState,
} from "../features/session/workstreamTabs";
import { WorkstreamDocsPanel } from "../features/workstreams/WorkstreamDocsPanel";
import { ProjectBoard } from "../features/board/ProjectBoard";
import { AttachPrDialog } from "../features/workstreams/AttachPrDialog";
import { useBootstrap, useDetachPr, useSetWorkstreamStage, useWorkstreams } from "../lib/queries";
import { basename } from "../lib/path";
import { clamp, startEdgeDrag } from "../lib/resize";
import { DIFF_SIDEBAR_MAX, DIFF_SIDEBAR_MIN, useUiStore } from "../lib/uiStore";

/** A doc attached as context to ONE agent tab. */
export type AttachedDoc = {
  path: string;
  role: string;
  /** Auto-suggested by the pane's mode (not user-attached). */
  suggested?: boolean;
};

/** A chat tab inside ONE workstream. Each workstream owns its own set.
 *  Context docs are PER TAB — each agent carries its own context. Once an
 *  agent session is started from this tab, `sessionId` binds the tab to that
 *  live (or terminal) session; its transcript lives in the session store, so
 *  it survives tab/workstream switches and re-attaches on demand. */
export type ShellTab = {
  id: number;
  title: string;
  /** Set once the title is "claimed" — either auto-generated from the first
   *  prompt or manually renamed — so auto-titling never overwrites it (the
   *  "one-time only" policy). Undefined on a fresh `Agent N` tab. */
  titleLocked?: boolean;
  mode: StageKey;
  draft: string;
  docs: AttachedDoc[];
  /** Paths of suggested docs the user explicitly dismissed in this pane; never
   *  re-suggested for the lifetime of the tab even if the mode switches back.
   *  Also covers auto-captured handoff docs: once detached, that path stays
   *  detached so the in-session re-write doesn't keep re-attaching it. */
  dismissedSuggestions?: string[];
  /** The user dismissed the "context is filling up — create a handoff?" banner
   *  for this tab. Only suppresses the suggestion banner; an actual handoff doc
   *  (auto-captured into [`docs`]) still surfaces the ready-to-resume banner. */
  handoffNudgeDismissed?: boolean;
  /** Files/screenshots staged in this tab's composer (dropped, pasted, or picked
   *  from the file dialog). Volatile sources are copied into the worktree's
   *  `.hyprlayer/attachments/` first; sent with the next turn, then cleared. */
  attachments: Attachment[];
  /** Model + reasoning-effort keys chosen for the NEXT session this tab starts
   *  (see models.ts); fixed at spawn. Undefined → the defaults. */
  model?: string;
  effort?: string;
  /** Permission-mode key chosen for the NEXT session (see permissions.ts);
   *  fixed at spawn. Undefined → "auto" (the stage's default posture). */
  permissionMode?: string;
  /** Which harness the NEXT session runs on ("claude" | "opencode"); fixed at
   *  spawn, persisted so switching tabs doesn't reset it. Undefined → "claude". */
  harness?: string;
  /** OpenCode `provider/model` id for the NEXT session when `harness` is
   *  "opencode" (Claude's model lives in `model`). Undefined → the OpenCode default. */
  opencodeModel?: string;
  sessionId?: string;
  /** When set, this tab is a read-only markdown preview of a doc (its path),
   *  not an agent chat — opened from the diff sidebar, closed when done. */
  docPreviewPath?: string;
  /** When set, this tab is a rich diff of a worktree file (its path), against
   *  either its staged or working version. Opened from the Changes panel. */
  diffView?: { path: string; staged: boolean };
  /** When set, this tab is a read-only view of a worktree file (its path).
   *  Opened from the Files tree. */
  filePath?: string;
  /** When set, this tab is an interactive terminal (PTY) for the worktree.
   *  `terminalId` binds it to a live backend PTY once opened; absent until then. */
  terminalView?: { terminalId?: string };
  /** Which pane this shell lives in. Absent/"main" → the main pane; "right" →
   *  the side-by-side right split. */
  panel?: "main" | "right";
};

type ShellMenu = {
  shellId: number;
  x: number;
  y: number;
};

/** A read-only viewer tab (doc preview, diff, or file) rather than an agent
 *  chat — these carry no session, so the strip shows a doc icon and no dot. */
function isViewerShell(shell: ShellTab): boolean {
  return !!(shell.docPreviewPath || shell.diffView || shell.filePath);
}

/** An interactive terminal tab — has its own PTY, no session dot. */
function isTerminalShell(shell: ShellTab): boolean {
  return !!shell.terminalView;
}

/** An agent chat tab (the only kind with a composer) — not a viewer or terminal. */
function isAgentShell(shell: ShellTab): boolean {
  return !isViewerShell(shell) && !isTerminalShell(shell);
}

/** The disabled-reason shown when a still-working agent can't be archived yet. */
const AGENT_BUSY_HINT = "Agent is still working — stop it first.";

/** Snapshot an agent's lifecycle into a coarse archive-row label at archive time
 *  (the live entry isn't kept around while archived). */
function classifyArchivedStatus(state: SessionState | undefined): ArchivedStatus {
  if (!state) return "idle";
  if (state === "completed") return "done";
  if (isTerminal(state)) return "stopped";
  return "idle";
}

/** The tab's "×": archives an agent (keeping its session/branch/transcript)
 *  rather than ending it — closing moves to right-click → Close. Terminals and
 *  viewers just close. Soft-disabled (reactively) while the agent is working. */
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
  const agent = isAgentShell(shell);
  const busy = agent && session ? isBusy(session) : false;
  const label = agent ? (busy ? AGENT_BUSY_HINT : `Archive ${shell.title}`) : `Close ${shell.title}`;
  return (
    <button
      type="button"
      aria-disabled={busy}
      onClick={(e) => {
        e.stopPropagation();
        if (busy) return;
        if (agent) onArchive();
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

/**
 * Detach the doc at `path` from a shell. A manually-attached doc is just removed;
 * detaching a *suggested* chip also records the path in `dismissedSuggestions` so
 * the suggestion never re-adds for this pane. Pure: returns the next shell.
 */
function detachDoc(shell: ShellTab, path: string): ShellTab {
  const removed = shell.docs.find((d) => d.path === path);
  const docs = shell.docs.filter((d) => d.path !== path);
  // Suggested chips and auto-captured handoff docs both need to be remembered
  // as dismissed — they're re-added by their respective effects otherwise.
  const persistDismissal = Boolean(removed?.suggested) || removed?.role === "handoff";
  if (!persistDismissal) return { ...shell, docs };
  return {
    ...shell,
    docs,
    dismissedSuggestions: [...new Set([...(shell.dismissedSuggestions ?? []), path])],
  };
}

/**
 * Registers the project/session keyboard commands whose action closures live in
 * `ProjectPage`. Split into its own component so its hooks (notably `useSession`
 * for the focused pane) run unconditionally — `ProjectPage` itself has early
 * returns above where these closures are defined. Mounted only when a workstream
 * is open, so the commands are naturally unregistered (inert) otherwise.
 */
function ProjectCommands(props: {
  sessionId: string | undefined;
  onNewTab: () => void;
  onCloseTab: (() => void) | null;
  onToggleSplit: () => void;
  onNextTab: () => void;
  onPrevTab: () => void;
  onNewWorkstream: () => void;
  /** ⌘R: run the workstream's Run script, or stop it if already live. Null when
   *  no workstream is selected. */
  onToggleRun: (() => void) | null;
  /** Cycle the focused agent's stage, or null when there's no un-started agent
   *  tab in focus (a running agent's stage is fixed; terminals have none). */
  onCycleStage: (() => void) | null;
  /** Toggle the archived-agents pane, or null when no workstream is selected. */
  onToggleArchived: (() => void) | null;
}) {
  const session = useSession(props.sessionId);
  // "Stop turn" only makes sense while the focused pane's session is working /
  // blocked on the user; otherwise there's no turn to interrupt.
  const running = Boolean(props.sessionId && session && isBusy(session));
  const sid = props.sessionId;
  useCommand("tab.new", props.onNewTab);
  useCommand("tab.close", props.onCloseTab);
  useCommand("tab.toggleSplit", props.onToggleSplit);
  useCommand("tab.next", props.onNextTab);
  useCommand("tab.prev", props.onPrevTab);
  useCommand("ws.new", props.onNewWorkstream);
  useCommand("scripts.run", props.onToggleRun);
  useCommand("ws.cycleStage", props.onCycleStage);
  useCommand("view.toggleArchivedAgents", props.onToggleArchived);
  useCommand("session.stop", running && sid ? () => void stopSession(sid) : null);
  useCommand("session.clear", sid ? () => void killSession(sid) : null);
  return null;
}

export function ProjectPage() {
  const { projectPath } = useParams({ from: "/projects/$projectPath" });
  const bootstrap = useBootstrap();
  const workstreams = useWorkstreams(projectPath);
  const setStage = useSetWorkstreamStage(projectPath);

  const search = useSearch({ from: "/projects/$projectPath" });
  const navigate = useNavigate({ from: "/projects/$projectPath" });

  // Selection is DERIVED from the URL every render — never mirrored into
  // state. (A state mirror + URL canonicalizer once fought each other into
  // an infinite navigate loop when switching between two workstreams.)
  // When no ?item= is present, the project-root landing shows the board.
  const selectedId = search.item;

  // Shell tabs are PER WORKSTREAM (1 workstream -> N tabs), keyed by workstream
  // id so switching workstreams never mixes tabs. Held in a module store so a
  // tab's bound session (and its draft/docs) survives leaving this page.
  const tabsByWs = useWorkstreamTabsStore((s) => s.byWs);
  // The URL is the sole source of truth for which workstream is selected (no
  // localStorage fallback — the project root with no ?item= shows the board).
  // It's derived purely from the URL + the already-loaded list, so we compute it
  // here, above the dock-open selector — which then subscribes to just THIS
  // workstream's open flag rather than the whole dock map (the map changes on any
  // workstream's dock interaction, which would re-render this entire page).
  const views = (workstreams.data ?? []).filter((view) => view.item.status === "active");
  const activeView = selectedId ? (views.find((v) => v.item.id === selectedId) ?? null) : null;
  const wsId = activeView?.item.id ?? null;
  const dockOpen = useScriptsStore((s) => (wsId ? !!s.dock[wsId]?.open : false));
  const dockPosition = useScriptsStore((s) => s.dockPosition);
  const [shellMenu, setShellMenu] = useState<ShellMenu | null>(null);
  const [createOpen, setCreateOpen] = useState(false);
  // Board-card right-click menu (Attach/Change/Detach PR) and the attach dialog.
  const [cardMenu, setCardMenu] = useState<{ x: number; y: number; view: WorkstreamView } | null>(
    null,
  );
  const [attachPrTarget, setAttachPrTarget] = useState<WorkstreamView | null>(null);
  const detachPr = useDetachPr(cardMenu?.view.item.id ?? "");
  const [renameShellId, setRenameShellId] = useState<number | null>(null);
  const [renameValue, setRenameValue] = useState("");
  // Which shell's "+" opened the library picker modal (null = closed).
  const [docsModalShellId, setDocsModalShellId] = useState<number | null>(null);
  // Hidden state lives in a shared store so the top-bar toggle controls it.
  const diffHidden = useUiStore((state) => state.diffHidden);
  // Width lives in the same store so the Root header can inset its right-side
  // controls to the agent pane's right margin — and so dragging the divider
  // updates both the sidebar and the header in one place.
  const diffWidth = useUiStore((state) => state.diffWidth);
  const setDiffWidth = useUiStore((state) => state.setDiffWidth);
  // True only while the diff-sidebar resize handle is dragged — suppresses the
  // width transition so the panel tracks the cursor instead of trailing it.
  const [resizingDiff, setResizingDiff] = useState(false);
  // Right-split width as a fraction of the agent area (0..1). The split's
  // open/closed state and pane assignments persist in the tab store, but the
  // exact ratio resets to a sane default per visit.
  const [rightRatio, setRightRatio] = useState(0.5);
  // Which pane (if any) is maximized to fill the whole agent view, scoped to
  // the workstream it was triggered in so switching workstreams un-maximizes.
  // Ephemeral view state (a temporary "focus this terminal"), not persisted.
  const [maximized, setMaximized] = useState<{ wsId: string; pane: "main" | "right" } | null>(
    null,
  );
  const splitRowRef = useRef<HTMLDivElement>(null);
  // Tab drag (reorder within a pane + move across the split). The hook must run
  // before this component's early returns, so it reaches the action closures
  // (defined later) through a ref.
  const dragHandlersRef = useRef<TabDragHandlers>({ reorder: () => {}, move: () => {} });
  const { drag, dropPane, suppressClickRef, startDrag } = useTabDrag(dragHandlersRef);

  // ?create=true (sidebar "+") pops the dialog once, then leaves the URL.
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

  // The scripts dock is shown when the user has expanded it for this ws; its
  // position (bottom strip vs. right sidebar) is a global preference. (views/
  // activeView/wsId/dockOpen are computed above, with the store hooks.)
  const rightDock = !!activeView && dockOpen && dockPosition === "right";
  // The right sidebar's visibility is owned by its own toggle. A right-docked
  // scripts panel only renders while that sidebar is shown (the rail icon reveals
  // it when needed) — so the toggle still collapses the whole sidebar.
  const showAside = !diffHidden;
  // True when no workstream is selected — the project-overview board.
  const showBoard = selectedId === undefined;

  // Stale-strip: if the URL names a workstream that the SETTLED list doesn't
  // contain (archived/removed elsewhere), drop back to the board. Never fires
  // mid-refetch, where a just-created id can legitimately be absent for a moment.
  useEffect(() => {
    if (selectedId === undefined) return; // board: nothing to fix
    if (!workstreams.isSuccess || workstreams.isFetching) return; // wait for list to settle
    if (!views.some((v) => v.item.id === selectedId)) {
      void navigate({ search: (prev) => ({ ...prev, item: undefined }), replace: true });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedId, workstreams.isSuccess, workstreams.isFetching]);

  // Point the backend's live worktree watcher at the viewed workstream, so
  // external edits (an editor, a build, a git checkout) push a refresh of the
  // Changes panel + git status without waiting for window focus — the way the
  // agent and terminal panes already stream live.
  useEffect(() => {
    if (!wsId) return;
    void commands.watchWorktree(wsId);
  }, [wsId]);

  // Publish the on-screen workstream so the left-rail scripts icon can toggle its
  // dock; clear it when leaving the project view.
  useEffect(() => {
    setActiveWorkstream(wsId);
    return () => setActiveWorkstream(null);
  }, [wsId]);

  // Landing on a workstream — switching focus to it, or opening it fresh — drops
  // the cursor straight into its composer so the user can type immediately, no
  // click needed. We target the active tab of the focused pane when it's an
  // agent (terminals/viewers have no composer); the shell's SessionChat consumes
  // the one-shot focus signal once it's on screen. Keyed on `wsId` ONLY: this
  // must fire on a workstream switch, never on in-workstream tab edits — a
  // `tabsByWs` dependency would re-run on every keystroke (draft writes mutate
  // the strip) and yank the caret to the end mid-type.
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

  if (bootstrap.isLoading || workstreams.isLoading) return <PageSpinner />;
  if (bootstrap.isError) return <PageError error={bootstrap.error} />;
  if (workstreams.isError) return <PageError error={workstreams.error} />;

  // The bootstrap cache already names every project — no per-page
  // `storage info` subprocess needed just for the mapped name.
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

  // The active workstream's own tab strip (1 workstream -> N shells).
  const tabs = wsId ? (tabsByWs[wsId] ?? defaultTabState()) : null;
  const shells = tabs?.shells ?? [];
  // The split partitions a workstream's shells into the main pane and the
  // right pane by their `panel`; each pane tracks its own active shell.
  const isMainShell = (s: ShellTab) => (s.panel ?? "main") !== "right";
  const isRightShell = (s: ShellTab) => s.panel === "right";
  const mainShells = shells.filter(isMainShell);
  const rightShells = shells.filter(isRightShell);
  const rightSplit = (tabs?.rightSplit ?? false) && rightShells.length > 0;
  const activePane: "main" | "right" = tabs?.activePane ?? "main";
  // A maximized pane hides its sibling (and the sibling's tab strip) so the
  // active pane fills the agent view. Only meaningful while the split is open.
  const maxPane = maximized && maximized.wsId === wsId ? maximized.pane : null;
  const maxMain = maxPane === "main" && rightSplit;
  const maxRight = maxPane === "right" && rightSplit;
  const maximizedView = maxMain || maxRight;
  const activeShell = tabs
    ? (mainShells.find((s) => s.id === tabs.activeId) ?? mainShells[0] ?? null)
    : null;
  const rightActiveShell =
    rightShells.find((s) => s.id === tabs?.rightActiveId) ?? rightShells[0] ?? null;
  // The shell keyboard commands act on: the active tab of whichever pane holds
  // focus, so split-pane users stop/close the agent they're looking at.
  const focusedShell = activePane === "right" ? rightActiveShell : activeShell;
  // The shell whose composer "+" opened the library picker — the modal's
  // attach target, stable while the modal is open.
  const docsModalShell =
    docsModalShellId !== null ? (shells.find((s) => s.id === docsModalShellId) ?? null) : null;
  const contextShell = shellMenu ? shells.find((s) => s.id === shellMenu.shellId) : null;
  const archivedShells = tabs?.archived ?? [];
  const archiveViewOpen = tabs?.archiveOpen ?? false;
  // One toggle for both entry points (tab-bar button + keyboard command), so a
  // repeat flips the same state shut. Null with no workstream selected, which
  // leaves the keyboard command inert.
  const toggleArchived = wsId
    ? () => updateTabs((c) => ({ ...c, archiveOpen: !c.archiveOpen }))
    : null;
  // The right-clicked tab is the main pane's last agent, so closing or archiving
  // it would empty the pane — both are blocked.
  const contextIsLastMain = !!contextShell && isMainShell(contextShell) && mainShells.length === 1;
  // Why "Archive agent" is blocked for the right-clicked tab (null = allowed).
  // Read non-reactively: the menu is short-lived and a hook can't run here (the
  // early returns above forbid it).
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

  const updateTabs = (updater: (current: WsTabState) => WsTabState) => {
    if (!wsId) return;
    updateWorkstreamTabs((all) => {
      const current = all[wsId] ?? defaultTabState();
      // updateTabs is the funnel every tab action passes through, so clearing the
      // archived-agents view on refocus here means no caller has to remember to.
      return { ...all, [wsId]: clearArchiveOnRefocus(current, updater(current)) };
    });
  };
  // Replace one shell by id via a transform — the single "map the shells, touch
  // the matching one" primitive the per-shell mutators build on.
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
        | "sessionId"
        | "handoffNudgeDismissed"
        | "terminalView"
      >
    >,
  ) => modifyShell(shellId, (s) => ({ ...s, ...patch }));
  const setTerminalId = (shellId: number, terminalId: string) =>
    patchShell(shellId, { terminalView: { terminalId } });
  const selectShell = (shellId: number) => {
    updateTabs((current) => ({ ...current, activeId: shellId, activePane: "main" }));
  };
  const selectRightShell = (shellId: number) => {
    updateTabs((current) => ({ ...current, rightActiveId: shellId, activePane: "right" }));
  };
  const setActivePane = (pane: "main" | "right") => {
    updateTabs((current) => (current.activePane === pane ? current : { ...current, activePane: pane }));
  };
  // Ctrl+Tab / Ctrl+Shift+Tab: move to the next/previous agent tab within the
  // focused pane, wrapping at the ends (a no-op when that pane has one tab).
  const cycleTab = (dir: 1 | -1) => updateTabs((current) => cycleActiveTab(current, dir));
  const makeShell = (id: number, panel: "main" | "right"): ShellTab => ({
    id,
    title: `Agent ${id}`,
    mode: "general",
    draft: "",
    docs: [],
    attachments: [],
    panel,
  });
  // Append a fresh agent to the right pane and focus it (opening the split).
  const spawnRightShell = (current: WsTabState): WsTabState => {
    const id = nextShellId(current.shells, current.archived);
    return {
      ...current,
      shells: [...current.shells, makeShell(id, "right")],
      rightActiveId: id,
      rightSplit: true,
      activePane: "right",
    };
  };
  const addShell = () => {
    updateTabs((current) => {
      const id = nextShellId(current.shells, current.archived);
      return {
        ...current,
        shells: [...current.shells, makeShell(id, "main")],
        activeId: id,
        activePane: "main",
      };
    });
  };
  const addRightShell = () => updateTabs(spawnRightShell);
  // The split toggle in the tab strip: open the right pane (seeding a fresh
  // agent the first time) or hide it. Hiding keeps the right shells around.
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
  // The X on the right strip tears the split down: end its agents and drop the
  // right shells entirely (vs. the toggle, which just hides the pane).
  const closeRightSplit = () => {
    setMaximized(null);
    for (const s of rightShells) {
      if (s.sessionId) void killSession(s.sessionId);
      if (s.terminalView?.terminalId) void commands.terminalClose(s.terminalView.terminalId);
    }
    updateTabs((current) => ({
      ...current,
      shells: current.shells.filter(isMainShell),
      rightSplit: false,
      rightActiveId: undefined,
      activePane: "main",
    }));
  };
  const moveShellToPane = (shellId: number, toPane: "main" | "right") => {
    updateTabs((current) => {
      const shell = current.shells.find((s) => s.id === shellId);
      if (!shell || (shell.panel ?? "main") === toPane) return current;
      if (toPane === "right") {
        const mains = current.shells.filter(isMainShell);
        if (mains.length <= 1) return current; // never empty the main pane
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
  // Reorder a shell within its own pane to `toIndex` (an index among that
  // pane's shells). The other pane's shells keep their slots — we only shuffle
  // the dragged pane's members back into the positions they occupied.
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
      if (reordered.every((s, i) => s.id === members[i].id)) return current; // no-op
      let i = 0;
      return { ...current, shells: current.shells.map((s) => (inPane(s) ? reordered[i++] : s)) };
    });
  };
  // Keep the drag hook pointed at the latest action closures.
  dragHandlersRef.current = { reorder: reorderShellInPane, move: moveShellToPane };
  // Open (or re-focus) a read-only viewer tab in the main pane. `match` finds an
  // already-open tab for the same target so we re-focus instead of stacking
  // dupes; `fields` carries the discriminator + title for a fresh tab. All three
  // viewer kinds (doc preview, diff, file) go through this.
  const openViewerTab = (
    match: (shell: ShellTab) => boolean,
    title: string,
    fields: Partial<ShellTab>,
  ) => {
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
  };
  const openDocPreview = (path: string, title: string) =>
    openViewerTab((s) => s.docPreviewPath === path, title, { docPreviewPath: path });
  const openDiff = (path: string, staged: boolean) =>
    openViewerTab(
      (s) => s.diffView?.path === path && s.diffView?.staged === staged,
      basename(path),
      { diffView: { path, staged } },
    );
  const openFile = (path: string) =>
    openViewerTab((s) => s.filePath === path, basename(path), { filePath: path });
  const closeShell = (shellId: number) => {
    if (!tabs) return;
    const shell = tabs.shells.find((s) => s.id === shellId);
    if (!shell) return;
    // The main pane must always keep at least one shell.
    if (isMainShell(shell) && tabs.shells.filter(isMainShell).length === 1) return;
    // Closing ends the session — there's no End button; the window IS the handle.
    // (No-op if the session already ended or was never started.)
    if (shell.sessionId) void killSession(shell.sessionId);
    if (shell.terminalView?.terminalId) void commands.terminalClose(shell.terminalView.terminalId);
    updateTabs((current) => removeShellState(current, shellId));
  };
  // Archive an agent: lift its tab out of the strip but keep the session, branch,
  // worktree and transcript intact (the opposite of closeShell). The status
  // snapshot only labels the archive row; archiveShellState no-ops if it can't.
  const archiveShell = (shellId: number) => {
    const shell = shells.find((s) => s.id === shellId);
    if (!shell) return;
    const entry = shell.sessionId
      ? useSessionStore.getState().sessions[shell.sessionId]
      : undefined;
    const status = classifyArchivedStatus(entry?.state);
    updateTabs((current) => archiveShellState(current, shellId, status, Date.now()));
  };
  // Restore an archived agent and focus its composer — re-mounting SessionChat
  // re-attaches the live session or rehydrates it from the transcript.
  const unarchiveShell = (shellId: number) => {
    updateTabs((current) => unarchiveShellState(current, shellId));
    requestComposerFocus(shellId);
  };
  // Drop an archived entry for good. The branch/worktree/transcript live on the
  // backend and are untouched — only the resumable tab handle is forgotten.
  const removeArchivedShell = (shellId: number) =>
    updateTabs((current) => ({
      ...current,
      archived: (current.archived ?? []).filter((a) => a.shell.id !== shellId),
    }));
  const openDoc = (id: string) => {
    void navigate({ to: "/library", search: { doc: id } });
  };
  // Library attach/detach mutate the shell whose composer opened the modal.
  const attachDocToShell = (path: string, role: string) => {
    if (!docsModalShell) return;
    const docs = docsModalShell.docs.some((d) => d.path === path)
      ? docsModalShell.docs
      : [...docsModalShell.docs, { path, role }];
    patchShell(docsModalShell.id, { docs });
  };
  const detachDocFromShell = (path: string) => {
    if (!docsModalShell) return;
    patchShell(docsModalShell.id, { docs: docsModalShell.docs.filter((d) => d.path !== path) });
  };

  // The URL is the single source of truth; the canonicalize effect persists
  // the last-used id once the selection settles.
  const selectItem = (id: string) => {
    void navigate({
      search: (prev) => ({ ...prev, item: id }),
      replace: true,
    });
  };
  const changeShellMode = (shell: ShellTab, key: StageKey) => {
    if (!activeView || key === shell.mode) return;
    patchShell(shell.id, { mode: key });
    // The workstream's pipeline position follows the latest mode choice.
    setStage.mutate({ id: activeView.item.id, stage: key });
  };
  // Stage attachments (dropped, pasted, or dialog-picked) on a shell's composer,
  // deduped by path. Sent with the next turn (start or follow-up), then cleared.
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
  // Attach files (or, filtered to images, screenshots) to a shell via the native
  // OS file dialog. Images are copied into the worktree's attachments dir (same
  // as a drop); other files are referenced in place.
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
  // Reconcile the pane's *suggested* doc to `path` (or clear it when path is
  // null). Never touches user-attached docs (those have suggested !== true).
  const suggestDocToShell = (shellId: number, path: string | null, role: string) =>
    modifyShell(shellId, (s) => {
      const dismissed = s.dismissedSuggestions ?? [];
      const manual = s.docs.filter((d) => !d.suggested);
      const alreadyManual = path != null && manual.some((d) => d.path === path);
      const keepSuggested =
        path && !alreadyManual && !dismissed.includes(path)
          ? [{ path, role, suggested: true as const }]
          : [];
      return { ...s, docs: [...manual, ...keepSuggested] };
    });

  // The full set of SessionChat callbacks scoped to ONE shell, so the main and
  // right panes each drive their own agent independently.
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
      // Push the switch to the live session too: OpenCode takes a model per
      // prompt, so this retargets the next turn without a restart. A no-op on the
      // backend when the session isn't live — a fresh session picks up the shell
      // value at launch instead.
      if (shell.sessionId) void commands.setSessionModel(shell.sessionId, key);
    },
    onOpenHarnessSettings: () => void navigate({ to: "/settings", search: { section: "harnesses" } }),
    // Strip the auto-captured handoff chip(s) once the new session has started:
    // by then they've already ridden through `handleStart` as attachments, so the
    // bridge has served its purpose — keeping the chip would leave the ready
    // banner re-showing in the resumed session and block a future suggest fire.
    onSessionStarted: (sessionId: string) =>
      modifyShell(shell.id, (s) => ({
        ...s,
        sessionId,
        handoffNudgeDismissed: false,
        docs: s.docs.filter((d) => d.role !== "handoff"),
      })),
    onResetSession: () => patchShell(shell.id, { sessionId: undefined }),
    onOpenDoc: openDoc,
    // Detaching a suggested chip records the dismissal so it does not re-add;
    // detaching a manually-attached doc just removes it. See `detachDoc`.
    onDetachDoc: (path: string) => modifyShell(shell.id, (s) => detachDoc(s, path)),
    onSuggestDoc: (path: string | null, role: string) => suggestDocToShell(shell.id, path, role),
    onDismissHandoffNudge: () => patchShell(shell.id, { handoffNudgeDismissed: true }),
    // Append the handoff doc as a regular (non-suggested) chip so it sticks
    // through `handleClear` and rides as an attachment into the fresh session.
    // Deduped by path; the auto-attach effect in SessionChat is the only caller.
    onAttachHandoffDoc: (path: string) =>
      modifyShell(shell.id, (s) =>
        s.docs.some((d) => d.path === path)
          ? s
          : { ...s, docs: [...s.docs, { path, role: "handoff" }] },
      ),
    onBrowseDocs: () => setDocsModalShellId(shell.id),
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
    // A manual rename claims the title for good — lock it so auto-titling (which
    // only fires on an unlocked tab) can never overwrite the user's choice.
    patchShell(shellBeingRenamed.id, { title: nextTitle, titleLocked: true });
    closeRenameShell();
  };
  const copyShellTranscript = (shell: ShellTab) => {
    const body = shell.sessionId
      ? transcriptToText(shell.sessionId)
      : `Draft (no session started yet):\n${shell.draft || "(empty)"}`;
    const transcript = [
      `# ${activeView?.item.title ?? project.mappedName} — ${shell.title}`,
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
    startEdgeDrag(event, {
      startWidth: rightRatio * rowW,
      min: 280,
      max: Math.max(280, rowW - 280),
      // Dragging the divider left grows the right pane.
      direction: -1,
      onResize: (width) => setRightRatio(clamp(width / rowW, 0.2, 0.8)),
    });
  };
  // The diff sidebar's handle sits on its left edge, so dragging left grows it.
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

  // One tab pill, shared by both pane strips. `paneFocused` is true when this
  // pill's pane holds focus — only then does its active tab read as fully
  // selected, so across a split only one tab looks active at a time.
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
        // While dragging, the live-reordering tab's own slot reads as a clear
        // accent placeholder so it's obvious where the tab will land on drop.
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
          // Accent-tinted badge so a diff tab is obvious at a glance, distinct
          // from dim doc/file viewers and the hue-coded agent tabs.
          <span className="rounded bg-accent/15 p-0.5 text-accent" title="Diff view">
            <DiffIcon size={13} />
          </span>
        ) : isTerminalShell(shell) ? (
          <span className="text-ink-dim">
            <TerminalIcon size={13} />
          </span>
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
        {!isViewerShell(shell) && !isTerminalShell(shell) && (
          <SessionTabDot sessionId={shell.sessionId} />
        )}
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

  // The body of one pane: a doc preview, an agent chat, or a placeholder.
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
    return <SessionChat key={`${activeView.item.id}:${shell.id}`} {...shellChatProps(shell)} />;
  };

  // Board: render when no workstream is selected (project-root click).
  // All hooks above still run harmlessly with wsId === null.
  if (showBoard) {
    return (
      <>
        <ProjectBoard
          views={views}
          onOpen={(id) => selectItem(id)}
          onNewWorkstream={() => setCreateOpen(true)}
          onCardContextMenu={(view, x, y) => setCardMenu({ x, y, view })}
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
                  detachPr.mutate();
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
          open={createOpen}
          onClose={() => setCreateOpen(false)}
          onCreated={(view) => selectItem(view.item.id)}
        />
      </>
    );
  }

  // Guard: a selectedId that hasn't resolved yet (stale-strip in flight or
  // post-create refetch) shows a spinner rather than the empty-state card.
  if (selectedId && !activeView && workstreams.isFetching) return <PageSpinner />;

  return (
    <div className="flex h-full min-h-0 bg-surface">
      <ProjectCommands
        sessionId={focusedShell?.sessionId}
        onNewTab={addShell}
        onCloseTab={focusedShell ? () => closeShell(focusedShell.id) : null}
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
          // Only when an un-started agent tab holds focus: a live session's stage
          // is fixed at spawn, and terminal tabs have no stage. The composer
          // handles Shift+Tab itself while focused (see SessionChat); this is the
          // global path for when focus sits outside the composer.
          focusedShell && isAgentShell(focusedShell) && !focusedShell.sessionId
            ? () => changeShellMode(focusedShell, nextModeStage(focusedShell.mode))
            : null
        }
        onToggleArchived={toggleArchived}
      />
      <section className="flex min-w-0 flex-1 flex-col">
        <div className="flex min-h-0 flex-1">
          <main className="flex min-w-0 flex-1 flex-col">
            <div className="flex h-9 items-stretch border-b border-edge bg-panel">
              {/* Main-pane tab strip + the split / docs controls. */}
              <div
                data-pane="main"
                // pr-2.5 (not px-5) lines the rightmost control — the split
                // toggle, an h-8 button — up vertically with the title bar's
                // "changes sidebar" toggle above it (an h-7 button at pr-3); the
                // 2px-narrower title button needs 2px more inset to share a center.
                className={`flex min-w-0 flex-1 items-center justify-between gap-3 pl-5 pr-2.5 ${
                  maxRight ? "hidden" : ""
                } ${dropPane === "main" ? "bg-accent/10" : ""}`}
              >
                <ScrollableTabStrip
                  activeId={activeShell?.id ?? null}
                  trailing={
                    activeView ? (
                      <IconButton onClick={addShell} title="New tab" aria-label="New tab">
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
                    {/* Subtle archive entry-point: blends into the tab bar, shows a
                        count, and toggles the workstream's archived-agents view. */}
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
              {/* Right-pane tab strip, sized + bordered to sit over the right
                  pane below so the two read as one continuous column. */}
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
                      <IconButton onClick={addRightShell} title="New tab" aria-label="New tab">
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
                {isAgentShell(contextShell) && (
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
                  danger={isAgentShell(contextShell)}
                  disabled={contextIsLastMain}
                  onClick={() => {
                    closeShell(contextShell.id);
                    setShellMenu(null);
                  }}
                >
                  {isAgentShell(contextShell) ? "Close agent" : "Close tab"}
                </ContextMenuItem>
              </ContextMenu>
            )}
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
                      Each workstream gets its own branch and worktree, so agents can research,
                      plan, and implement without touching your checkout.
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

      {showAside && (
        <div
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize diff sidebar"
          className="relative z-10 -mx-1 w-2 shrink-0 cursor-col-resize bg-transparent transition-colors hover:bg-edge/60"
          onPointerDown={startDiffResize}
        />
      )}
      {/* Changes sidebar — width-animates between 0 and diffWidth so toggling it
          glides instead of snapping. overflow-hidden clips the fixed-width inner
          shell (pinned to the window edge) so its content never reflows; the
          transition is dropped while the resize handle is dragged. It is not
          reduced-motion-gated — Windows maps that to the often-off "Animation
          effects" toggle, which would otherwise snap it. */}
      <aside
        className={`relative shrink-0 overflow-hidden border-l border-edge bg-panel ${
          resizingDiff
            ? ""
            : "transition-[width] duration-200 ease-out"
        }`}
        style={{ width: showAside ? diffWidth : 0 }}
        inert={!showAside}
      >
        <div className="absolute inset-y-0 right-0 flex flex-col" style={{ width: diffWidth }}>
          <GitPanel view={activeView ?? null} />
          <div className="flex min-h-0 flex-1 flex-col">
            <ChangesPanel
              workstreamId={activeView?.item.id ?? null}
              onOpenDoc={openDocPreview}
              onOpenDiff={openDiff}
              onOpenFile={openFile}
            />
          </div>
          {rightDock && showAside && (
            <ScriptDock
              key={`${activeView!.item.id}:right`}
              workstreamId={activeView!.item.id}
              projectPath={projectPath}
              onEditScripts={() => openProjectSettings(projectPath)}
              orientation="right"
            />
          )}
          <PrChecksPanel view={activeView ?? null} />
        </div>
      </aside>

      {/* Library picker — opened from a composer's "+" menu. Centered modal so
          attaching context docs never replaces the agent view. */}
      <Dialog
        open={docsModalShellId !== null}
        onClose={() => setDocsModalShellId(null)}
        title="Attach context from library"
        className="!max-w-2xl"
      >
        <WorkstreamDocsPanel
          thoughtsProject={project.thoughtsProject}
          targetTitle={docsModalShell?.title ?? null}
          attachedPaths={(docsModalShell?.docs ?? []).map((d) => d.path)}
          onAttach={attachDocToShell}
          onDetach={detachDocFromShell}
          onOpenDoc={openDoc}
        />
      </Dialog>

      {/* Floating pill that trails the cursor while dragging a tab — styled as a
          "lifted" tab (accent ring, lift shadow, slight scale) so the drag reads
          clearly as carrying that tab to a new spot. */}
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
