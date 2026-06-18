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

import { DiffIcon, DocIcon, PlusIcon, SplitIcon, XIcon } from "../components/icons";
import { CreateWorkstreamDialog } from "../features/workstreams/CreateWorkstreamDialog";
import { ChangesPanel } from "../features/workstreams/ChangesPanel";
import { PrChecksPanel } from "../features/workstreams/PrChecksPanel";
import { DiffTab } from "../features/workstreams/DiffTab";
import { FileViewerTab } from "../features/workstreams/FileViewerTab";
import { SessionChat, SessionTabDot, SessionTabIcon } from "../features/session/SessionChat";
import { DocPreviewTab } from "../features/session/DocPreviewTab";
import { useTabDrag, type TabDragHandlers } from "../features/session/useTabDrag";
import { killSession, transcriptToText } from "../features/session/sessionStore";
import {
  importDroppedPaths,
  IMAGE_EXTENSIONS,
  type Attachment,
} from "../features/session/attachments";
import { stageLabel, type StageKey } from "../features/session/stages";
import {
  defaultTabState,
  nextShellId,
  updateWorkstreamTabs,
  useWorkstreamTabsStore,
  type WsTabState,
} from "../features/session/workstreamTabs";
import { WorkstreamDocsPanel } from "../features/workstreams/WorkstreamDocsPanel";
import { useBootstrap, useSetWorkstreamStage, useWorkstreams } from "../lib/queries";
import { basename } from "../lib/path";
import { clamp, startEdgeDrag } from "../lib/resize";
import { useUiStore } from "../lib/uiStore";

/** A doc attached as context to ONE agent tab. */
export type AttachedDoc = {
  path: string;
  role: string;
};

/** A chat tab inside ONE workstream. Each workstream owns its own set.
 *  Context docs are PER TAB — each agent carries its own context. Once an
 *  agent session is started from this tab, `sessionId` binds the tab to that
 *  live (or terminal) session; its transcript lives in the session store, so
 *  it survives tab/workstream switches and re-attaches on demand. */
export type ShellTab = {
  id: number;
  title: string;
  mode: StageKey;
  draft: string;
  docs: AttachedDoc[];
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

export function ProjectPage() {
  const { projectPath } = useParams({ from: "/projects/$projectPath" });
  const bootstrap = useBootstrap();
  const workstreams = useWorkstreams(projectPath);
  const setStage = useSetWorkstreamStage(projectPath);

  const search = useSearch({ from: "/projects/$projectPath" });
  const navigate = useNavigate({ from: "/projects/$projectPath" });

  const storageKey = `hyprlayer:last-workstream:${projectPath}`;
  // Selection is DERIVED from the URL every render — never mirrored into
  // state. (A state mirror + URL canonicalizer once fought each other into
  // an infinite navigate loop when switching between two workstreams.)
  const activeItemId = search.item ?? localStorage.getItem(storageKey);

  // Shell tabs are PER WORKSTREAM (1 workstream -> N tabs), keyed by workstream
  // id so switching workstreams never mixes tabs. Held in a module store so a
  // tab's bound session (and its draft/docs) survives leaving this page.
  const tabsByWs = useWorkstreamTabsStore((s) => s.byWs);
  const [shellMenu, setShellMenu] = useState<ShellMenu | null>(null);
  const [createOpen, setCreateOpen] = useState(false);
  const [renameShellId, setRenameShellId] = useState<number | null>(null);
  const [renameValue, setRenameValue] = useState("");
  // Which shell's "+" opened the library picker modal (null = closed).
  const [docsModalShellId, setDocsModalShellId] = useState<number | null>(null);
  // Hidden state lives in a shared store so the top-bar toggle controls it.
  const diffHidden = useUiStore((state) => state.diffHidden);
  // Width lives in the same store so the Root header can inset its right-side
  // controls to the agent pane's right margin (and track this if it ever
  // becomes drag-resizable).
  const diffWidth = useUiStore((state) => state.diffWidth);
  // Right-split width as a fraction of the agent area (0..1). The split's
  // open/closed state and pane assignments persist in the tab store, but the
  // exact ratio resets to a sane default per visit.
  const [rightRatio, setRightRatio] = useState(0.5);
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

  // Hooks live above the early returns. Selection resolves to the URL's
  // ?item= when it names a live workstream, else last-used, else the first.
  const views = (workstreams.data ?? []).filter((view) => view.item.status === "active");
  const activeView = views.find((view) => view.item.id === activeItemId) ?? views[0] ?? null;
  const wsId = activeView?.item.id ?? null;

  // Canonicalize the URL to the resolved selection. A valid ?item= always
  // resolves to itself (no rewrite, no loop); we only write when the param
  // is missing, or names something the SETTLED list doesn't contain (e.g.
  // archived elsewhere) — never mid-refetch, where a just-created id can
  // legitimately be absent for a moment.
  useEffect(() => {
    if (!wsId) return;
    localStorage.setItem(storageKey, wsId);
    const stale =
      search.item !== undefined &&
      search.item !== wsId &&
      workstreams.isSuccess &&
      !workstreams.isFetching;
    if (search.item === undefined || stale) {
      void navigate({
        search: (prev) => ({ ...prev, item: wsId }),
        replace: true,
      });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wsId, search.item, storageKey, workstreams.isSuccess, workstreams.isFetching]);

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
  const activeShell = tabs
    ? (mainShells.find((s) => s.id === tabs.activeId) ?? mainShells[0] ?? null)
    : null;
  const rightActiveShell =
    rightShells.find((s) => s.id === tabs?.rightActiveId) ?? rightShells[0] ?? null;
  // The shell whose composer "+" opened the library picker — the modal's
  // attach target, stable while the modal is open.
  const docsModalShell =
    docsModalShellId !== null ? (shells.find((s) => s.id === docsModalShellId) ?? null) : null;
  const contextShell = shellMenu ? shells.find((s) => s.id === shellMenu.shellId) : null;
  const shellBeingRenamed = renameShellId
    ? shells.find((s) => s.id === renameShellId)
    : null;

  const updateTabs = (updater: (current: WsTabState) => WsTabState) => {
    if (!wsId) return;
    updateWorkstreamTabs((all) => ({ ...all, [wsId]: updater(all[wsId] ?? defaultTabState()) }));
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
        | "mode"
        | "draft"
        | "docs"
        | "attachments"
        | "model"
        | "effort"
        | "permissionMode"
        | "sessionId"
      >
    >,
  ) => modifyShell(shellId, (s) => ({ ...s, ...patch }));
  const selectShell = (shellId: number) => {
    updateTabs((current) => ({ ...current, activeId: shellId, activePane: "main" }));
  };
  const selectRightShell = (shellId: number) => {
    updateTabs((current) => ({ ...current, rightActiveId: shellId, activePane: "right" }));
  };
  const setActivePane = (pane: "main" | "right") => {
    updateTabs((current) => (current.activePane === pane ? current : { ...current, activePane: pane }));
  };
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
    const id = nextShellId(current.shells);
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
      const id = nextShellId(current.shells);
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
    for (const s of rightShells) if (s.sessionId) void killSession(s.sessionId);
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
      const nextId = nextShellId(current.shells);
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
    const onRight = isRightShell(shell);
    // The main pane must always keep at least one shell.
    if (!onRight && tabs.shells.filter(isMainShell).length === 1) return;
    // Closing the tab ends its session — there's no End button; the window IS
    // the handle. (No-op if the session already ended or was never started.)
    if (shell.sessionId) void killSession(shell.sessionId);
    updateTabs((current) => {
      const remaining = current.shells.filter((s) => s.id !== shellId);
      if (onRight) {
        const rights = remaining.filter(isRightShell);
        return {
          ...current,
          shells: remaining,
          rightActiveId:
            current.rightActiveId === shellId
              ? rights[rights.length - 1]?.id
              : current.rightActiveId,
          rightSplit: rights.length > 0 ? current.rightSplit : false,
          activePane: rights.length > 0 ? current.activePane : "main",
        };
      }
      const mains = remaining.filter(isMainShell);
      const activeId =
        current.activeId === shellId
          ? (mains[mains.length - 1]?.id ?? current.activeId)
          : current.activeId;
      return { ...current, shells: remaining, activeId };
    });
  };
  const openDoc = (id: string) => {
    void navigate({ to: "/library/doc", search: { id, mode: "preview" } });
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
    onSessionStarted: (sessionId: string) => patchShell(shell.id, { sessionId }),
    onResetSession: () => patchShell(shell.id, { sessionId: undefined }),
    onOpenDoc: openDoc,
    onDetachDoc: (path: string) =>
      patchShell(shell.id, { docs: shell.docs.filter((d) => d.path !== path) }),
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
    patchShell(shellBeingRenamed.id, { title: nextTitle });
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
        drag?.id === shell.id ? "opacity-40" : ""
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
          <span className="text-ink-dim" title="Diff view">
            <DiffIcon size={13} />
          </span>
        ) : isViewerShell(shell) ? (
          <span className="text-ink-dim">
            <DocIcon size={13} />
          </span>
        ) : (
          <SessionTabIcon mode={shell.mode} sessionId={shell.sessionId} />
        )}
        <span className="max-w-[160px] truncate" title={shell.title}>
          {shell.title}
        </span>
        {!isViewerShell(shell) && <SessionTabDot sessionId={shell.sessionId} />}
      </span>
      {opts.closable && (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            closeShell(shell.id);
          }}
          className="mr-1 inline-flex h-5 w-5 items-center justify-center rounded text-ink-dim hover:bg-edge hover:text-ink"
          title={`Close ${shell.title}`}
          aria-label={`Close ${shell.title}`}
        >
          <XIcon />
        </button>
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
    return <SessionChat key={`${activeView.item.id}:${shell.id}`} {...shellChatProps(shell)} />;
  };

  return (
    <div className="flex h-full min-h-0 bg-surface">
      <section className="flex min-w-0 flex-1 flex-col">
        <div className="flex min-h-0 flex-1">
          <main className="flex min-w-0 flex-1 flex-col">
            <div className="flex h-9 items-stretch border-b border-edge bg-panel">
              {/* Main-pane tab strip + the split / docs controls. */}
              <div
                data-pane="main"
                className={`flex min-w-0 flex-1 items-center justify-between gap-3 px-5 ${
                  dropPane === "main" ? "bg-accent/10" : ""
                }`}
              >
                <div className="no-scrollbar flex min-w-0 items-center gap-1 overflow-x-auto">
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
                  {activeView && (
                    <IconButton onClick={addShell} title="New tab" aria-label="New tab">
                      <PlusIcon />
                    </IconButton>
                  )}
                </div>
                {activeView && (
                  <IconButton
                    onClick={toggleRightSplit}
                    active={rightSplit}
                    title={rightSplit ? "Close split" : "Split agent right"}
                    aria-label={rightSplit ? "Close split" : "Split agent right"}
                    className="shrink-0"
                  >
                    <SplitIcon className={rightSplit ? "text-accent" : undefined} />
                  </IconButton>
                )}
              </div>
              {/* Right-pane tab strip, sized + bordered to sit over the right
                  pane below so the two read as one continuous column. */}
              {activeView && rightSplit && (
                <div
                  data-pane="right"
                  className={`flex shrink-0 items-center gap-1 border-l border-edge px-3 ${
                    dropPane === "right" ? "bg-accent/10" : ""
                  }`}
                  style={{ width: `${rightRatio * 100}%` }}
                >
                  <div className="no-scrollbar flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
                    {rightShells.map((shell) =>
                      renderTabPill(shell, {
                        active: shell.id === rightActiveShell?.id,
                        paneFocused: activePane === "right",
                        pane: "right",
                        onSelect: () => selectRightShell(shell.id),
                        closable: true,
                      }),
                    )}
                    <IconButton onClick={addRightShell} title="New tab" aria-label="New tab">
                      <PlusIcon />
                    </IconButton>
                  </div>
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
                <ContextMenuItem
                  disabled={(contextShell.panel ?? "main") !== "right" && mainShells.length === 1}
                  onClick={() => {
                    closeShell(contextShell.id);
                    setShellMenu(null);
                  }}
                >
                  Close tab
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
              <div ref={splitRowRef} className="flex min-h-0 flex-1">
                <div
                  data-pane="main"
                  className={`relative flex min-h-0 min-w-0 flex-1 flex-col ${
                    dropPane === "main" ? "ring-2 ring-inset ring-accent/60" : ""
                  }`}
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
                      className="relative z-10 -mx-1 w-2 shrink-0 cursor-col-resize bg-transparent transition-colors hover:bg-edge/60"
                      onPointerDown={startSplitResize}
                    />
                    <div
                      data-pane="right"
                      className={`relative flex min-h-0 flex-col border-l border-edge ${
                        dropPane === "right" ? "ring-2 ring-inset ring-accent/60" : ""
                      }`}
                      style={{ width: `${rightRatio * 100}%` }}
                      onPointerDownCapture={() => {
                        if (activePane !== "right") setActivePane("right");
                      }}
                    >
                      {renderPaneContent(rightActiveShell)}
                    </div>
                  </>
                )}
              </div>
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
          </main>
        </div>
      </section>

      {!diffHidden && (
        <>
          <aside
            className="flex min-w-0 shrink-0 flex-col border-l border-edge bg-panel"
            style={{ width: diffWidth }}
          >
            <div className="flex min-h-0 flex-1 flex-col">
              <ChangesPanel
                workstreamId={activeView?.item.id ?? null}
                onOpenDoc={openDocPreview}
                onOpenDiff={openDiff}
                onOpenFile={openFile}
              />
            </div>
            <PrChecksPanel view={activeView ?? null} />
          </aside>
        </>
      )}

      {/* Library picker — opened from a composer's "+" menu. Centered modal so
          attaching context docs never replaces the agent view. */}
      <Dialog
        open={docsModalShellId !== null}
        onClose={() => setDocsModalShellId(null)}
        title="Attach context from library"
        className="max-w-2xl"
      >
        <WorkstreamDocsPanel
          mappedName={project.mappedName}
          targetTitle={docsModalShell?.title ?? null}
          attachedPaths={(docsModalShell?.docs ?? []).map((d) => d.path)}
          onAttach={attachDocToShell}
          onDetach={detachDocFromShell}
          onOpenDoc={openDoc}
        />
      </Dialog>

      {/* Floating label that trails the cursor while dragging a tab. */}
      {drag && (
        <div
          className="pointer-events-none fixed z-50 rounded-md border border-edge bg-panel-2 px-2 py-1 text-xs font-medium text-ink shadow-lg"
          style={{ left: drag.x + 12, top: drag.y + 12 }}
        >
          {drag.title}
        </div>
      )}
    </div>
  );
}
