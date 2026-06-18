import {
  Link,
  Outlet,
  useCanGoBack,
  useNavigate,
  useRouter,
  useRouterState,
} from "@tanstack/react-router";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react";
import { openPath } from "@tauri-apps/plugin-opener";

import {
  useAllWorkstreams,
  useArchiveWorkstream,
  useBootstrap,
  useEditorTargets,
  useWorkstreamPr,
} from "../lib/queries";
import { useElementWidth } from "../lib/useElementWidth";
import { commitWorkstream, shipWorkstream } from "../features/session/sessionStore";
import { WorkstreamActivityDot } from "../features/session/WorkstreamActivity";
import { startEdgeDrag } from "../lib/resize";
import { IS_MACOS } from "../lib/platform";
import { useUiStore } from "../lib/uiStore";
import { useCommand } from "../lib/keybindings/registry";
import { useKeybindings } from "../lib/keybindings/useKeybindings";
import type { Overrides } from "../lib/keybindings/resolve";
import { ContextMenu, ContextMenuItem, Spinner } from "../components/ui";
import { RepoAvatar } from "../components/RepoAvatar";
import {
  BookIcon,
  ChevronDownIcon,
  ChevronRightIcon,
  ClipboardIcon,
  CodeIcon,
  FolderIcon,
  GitCommitIcon,
  GitPullRequestIcon,
  PlusIcon,
  TerminalIcon,
} from "../components/icons";
import type { EditorTarget } from "../lib/bindings";
import { ArchiveWorkstreamDialog } from "../features/workstreams/ArchiveWorkstreamDialog";
import { PrStatusBar } from "../features/workstreams/PrStatusBar";
import { RenameWorkstreamDialog } from "../features/workstreams/RenameWorkstreamDialog";
import { AddProjectDialog } from "../features/projects/AddProjectDialog";
import { ProjectSettingsDialog } from "../features/projects/ProjectSettingsDialog";
import { UpdatePill } from "../features/system/updates";
import type { Project, WorkstreamView } from "../lib/bindings";

// One-shot guard (module scope, so it survives re-renders but resets on a cold
// app load): on launch we land on the dashboard even if the webview restored a
// deeper route from the previous session.
let launchedToHome = false;

// Stable identity for "no overrides yet" so the dispatcher's memo/effect doesn't
// churn on every Root render before settings have loaded a keybindings section.
const EMPTY_OVERRIDES: Overrides = {};

const PROJECT_SIDEBAR_COLLAPSED = 56;
const PROJECT_SIDEBAR_MIN = 180;
const PROJECT_SIDEBAR_MAX = 360;
// macOS only: keep the header's left content clear of the overlaid traffic
// lights + the collapse button. Other platforms don't float controls here.
const TOP_CHROME_MIN_CONTENT_OFFSET = 148;

// Cross-platform vector icons (Lucide family / native-style), currentColor so
// they theme with the bar and stay pixel-crisp at every DPI on macOS + Windows.
function PanelLeftIcon() {
  return (
    <svg aria-hidden="true" width="18" height="18" viewBox="0 0 24 24" fill="none">
      <rect x="3" y="4.5" width="18" height="15" rx="2.5" stroke="currentColor" strokeWidth="1.8" />
      <path
        d="M5.5 4.5H9v15H5.5A2.5 2.5 0 0 1 3 17V7a2.5 2.5 0 0 1 2.5-2.5Z"
        fill="currentColor"
        opacity="0.9"
      />
    </svg>
  );
}

function PanelRightIcon() {
  return (
    <svg aria-hidden="true" width="18" height="18" viewBox="0 0 24 24" fill="none">
      <rect x="3" y="4.5" width="18" height="15" rx="2.5" stroke="currentColor" strokeWidth="1.8" />
      <path
        d="M18.5 4.5H15v15h3.5A2.5 2.5 0 0 0 21 17V7a2.5 2.5 0 0 0-2.5-2.5Z"
        fill="currentColor"
        opacity="0.9"
      />
    </svg>
  );
}

function GitBranchIcon() {
  return (
    <svg
      aria-hidden="true"
      width="13"
      height="13"
      viewBox="0 0 24 24"
      fill="none"
      className="shrink-0"
    >
      <circle cx="6" cy="5" r="2.3" stroke="currentColor" strokeWidth="1.8" />
      <circle cx="6" cy="19" r="2.3" stroke="currentColor" strokeWidth="1.8" />
      <circle cx="18" cy="8" r="2.3" stroke="currentColor" strokeWidth="1.8" />
      <path d="M6 7.5v9" stroke="currentColor" strokeWidth="1.8" />
      <path
        d="M18 10.5c0 3-2.5 4.5-5.5 4.5H9.5"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
      />
    </svg>
  );
}

function SettingsIcon({ size = 18 }: { size?: number }) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
    </svg>
  );
}

function HomeIcon() {
  return (
    <svg
      aria-hidden="true"
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M3 10.5 12 3l9 7.5" />
      <path d="M5 9.5V20a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V9.5" />
      <path d="M9.5 21v-6h5v6" />
    </svg>
  );
}

function decodePathSegment(value: string): string {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

function routeProjectPath(pathname: string): string | null {
  const prefix = "/projects/";
  if (!pathname.startsWith(prefix)) return null;
  const segment = pathname.slice(prefix.length).split("/")[0];
  return segment ? decodePathSegment(segment) : null;
}

function routeSection(pathname: string): string {
  if (pathname.startsWith("/projects/") && pathname.endsWith("/docs")) return "Docs";
  if (pathname.startsWith("/projects/")) return "Agent";
  if (pathname.startsWith("/library/doc")) return "Document";
  if (pathname.startsWith("/library")) return "Library";
  if (pathname.startsWith("/settings")) return "Settings";
  if (pathname === "/") return "Home";
  return "Workspace";
}

const topIconClass =
  "inline-flex h-7 w-7 items-center justify-center rounded-md text-ink-dim hover:bg-edge/60 hover:text-ink";

/** Bottom-bar icon button (Library / Settings). */
const footerIconClass = (active: boolean): string =>
  `inline-flex h-8 w-8 items-center justify-center rounded-md hover:bg-edge/60 hover:text-ink ${
    active ? "bg-edge text-ink" : "text-ink-dim"
  }`;

/** Conductor-style status chip for a sidebar workstream row: added/deleted
 *  line counts (matching the diff panel), falling back to an ahead-of-base
 *  indicator once the working tree is clean but commits are unpushed. */
function WorkstreamRowChip({ view }: { view: WorkstreamView }) {
  const { additions, deletions, dirty, ahead } = view.git;
  if (additions > 0 || deletions > 0) {
    return (
      <span className="flex shrink-0 items-center gap-1 font-mono text-xs font-medium">
        {additions > 0 && <span className="text-ok">+{additions}</span>}
        {deletions > 0 && <span className="text-bad">-{deletions}</span>}
      </span>
    );
  }
  // Changes with no line delta (binary edits, mode/rename only) still count.
  if (dirty > 0) {
    return <span className="shrink-0 text-xs font-medium text-ink-dim">●{dirty}</span>;
  }
  if (ahead > 0) {
    return <span className="shrink-0 text-xs font-medium text-accent">↑{ahead}</span>;
  }
  return null;
}

/** A row/trigger icon for a detected app: its real rendered icon when the
 *  backend could rasterize one, otherwise a glyph standing in for its kind.
 *  App icons are full-bleed squircles, so they read heavier than the thin
 *  stroke glyphs beside them — render them at the exact target size (no
 *  upscaling) so they sit at the same optical weight. */
function appIcon(app: EditorTarget, size = 16): ReactNode {
  if (app.iconDataUrl) {
    return (
      <img
        src={app.iconDataUrl}
        alt=""
        className="block shrink-0 rounded-[4px]"
        style={{ width: size, height: size }}
      />
    );
  }
  switch (app.kind) {
    case "files":
      return <FolderIcon size={size} />;
    case "terminal":
      return <TerminalIcon size={size} />;
    default:
      return <CodeIcon size={size} />;
  }
}

/** Top-bar dropdown that opens the active worktree (or project) directory in
 *  any installed editor / terminal — detected by the backend so only apps the
 *  user actually has are listed — plus a Copy path fallback. Sits just left of
 *  the changes-sidebar toggle. */
function OpenInMenu({ dir }: { dir: string }) {
  const [open, setOpen] = useState(false);
  const targets = useEditorTargets();

  const items = useMemo(() => {
    const apps = (targets.data ?? []).map((app) => ({
      key: app.id,
      label: app.name,
      icon: appIcon(app),
      run: (): void => {
        // Best-effort: a wrong/uninstalled handler shouldn't surface a scary
        // error for a convenience action, but log so failures are debuggable.
        const p = app.openWith ? openPath(dir, app.openWith) : openPath(dir);
        void p.catch((e) => console.error(`Open in ${app.name} failed`, e));
      },
    }));
    return [
      ...apps,
      {
        key: "copy-path",
        label: "Copy path",
        icon: <ClipboardIcon size={16} />,
        run: (): void => {
          void navigator.clipboard.writeText(dir);
        },
      },
    ];
  }, [targets.data, dir]);

  // While open, number keys (1–9) trigger the matching row, mirroring the
  // accelerators shown at the right of each item; Escape closes.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
        return;
      }
      const n = Number(e.key);
      if (Number.isInteger(n) && n >= 1 && n <= Math.min(items.length, 9)) {
        e.preventDefault();
        setOpen(false);
        items[n - 1].run();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, items]);

  // The trigger wears the default editor's icon (first detected editor, else
  // the first openable app), Conductor-style, so it reads as "open in editor".
  const apps = targets.data ?? [];
  const defaultApp = apps.find((a) => a.kind === "editor") ?? apps.find((a) => a.id !== "files");

  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="inline-flex h-7 items-center gap-0.5 rounded-md pl-1.5 pr-1 text-ink-dim hover:bg-edge/60 hover:text-ink"
        title="Open in editor or terminal"
        aria-label="Open working directory in editor or terminal"
        aria-haspopup="menu"
        aria-expanded={open}
      >
        {defaultApp ? appIcon(defaultApp, 16) : <CodeIcon size={16} />}
        <ChevronDownIcon size={12} className="text-ink-dim/80" />
      </button>
      {open && (
        <>
          <div className="fixed inset-0 z-20" onClick={() => setOpen(false)} aria-hidden="true" />
          <div className="absolute right-0 top-full z-30 mt-1.5 min-w-[220px] overflow-hidden rounded-lg border border-edge bg-panel py-1 shadow-xl">
            {items.map((it, i) => (
              <button
                key={it.key}
                type="button"
                onClick={() => {
                  setOpen(false);
                  it.run();
                }}
                className="flex w-full items-center gap-2.5 px-3 py-1.5 text-left text-xs text-ink-dim hover:bg-edge/50 hover:text-ink"
              >
                <span className="shrink-0">{it.icon}</span>
                <span className="flex-1 font-medium">{it.label}</span>
                {i < 9 && (
                  <span className="shrink-0 font-mono text-[10px] text-ink-dim/60">{i + 1}</span>
                )}
              </button>
            ))}
          </div>
        </>
      )}
    </div>
  );
}

export function Root() {
  const bootstrap = useBootstrap();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const locationSearch = useRouterState({ select: (s) => s.location.search }) as {
    item?: string;
  };
  const [collapsed, setCollapsed] = useState(false);
  // Default to the widest we allow; the user can drag it narrower if desired.
  const [sidebarWidth, setSidebarWidth] = useState(PROJECT_SIDEBAR_MAX);
  const [shipping, setShipping] = useState(false);
  const [committing, setCommitting] = useState(false);

  const projects = bootstrap.data?.projects ?? [];
  const configReady = bootstrap.data?.config.state === "ready";
  const currentProjectPath = routeProjectPath(pathname);
  const currentProject = currentProjectPath
    ? projects.find((project) => project.path === currentProjectPath)
    : null;
  const currentSection = routeSection(pathname);

  // The sidebar tree: every project's active workstreams, nested beneath it.
  const allWorkstreams = useAllWorkstreams(projects.length > 0);
  const itemsByProject = new Map<string, WorkstreamView[]>();
  for (const view of allWorkstreams.data ?? []) {
    const group = itemsByProject.get(view.item.projectPath) ?? [];
    group.push(view);
    itemsByProject.set(view.item.projectPath, group);
  }
  const activeWorkstreamId = currentProjectPath ? locationSearch.item : undefined;
  const activeWorkstream = activeWorkstreamId
    ? (allWorkstreams.data ?? []).find((view) => view.item.id === activeWorkstreamId)
    : undefined;
  // Breadcrumb: prefer the selected workstream's name over the generic
  // section label on project routes.
  const crumb =
    currentSection === "Agent" ? (activeWorkstream?.item.title ?? "Workstreams") : currentSection;

  // A single contextual Back affordance: only on secondary views (library,
  // settings, docs, setup) so users can return to the agent view without
  // re-clicking the worktree in the sidebar. Never shown on the agent view
  // itself, and only when there is actual history to return to.
  const router = useRouter();
  const canGoBack = useCanGoBack();

  // Right-click menu + archive flow for the worktree rows. The view itself
  // carries its project path, so the targets are just the view.
  const navigate = useNavigate();

  // Toggling a "global destination" icon (Library / Settings / project docs)
  // while already there backs out to the previous view — or runs `fallback`
  // when this view was the entry point — instead of being a no-op the user has
  // to escape manually. (This replaces the header's dedicated Back button.)
  const closeOrBack = (fallback: () => void) => {
    if (canGoBack) router.history.back();
    else fallback();
  };
  const toggleNav = (target: "/library" | "/settings") => {
    if (pathname.startsWith(target)) closeOrBack(() => void navigate({ to: "/" }));
    else void navigate({ to: target });
  };
  const toggleProjectDocs = (projectPath: string) => {
    const onDocs = pathname.startsWith("/projects/") && pathname.endsWith("/docs");
    if (onDocs) {
      closeOrBack(() =>
        void navigate({ to: "/projects/$projectPath", params: { projectPath }, search: {} }),
      );
    } else {
      void navigate({ to: "/projects/$projectPath/docs", params: { projectPath } });
    }
  };

  // Make the dashboard the launch view: once per cold start, if we opened on a
  // restored deeper route, redirect home. In-session navigation is untouched.
  useEffect(() => {
    if (launchedToHome) return;
    launchedToHome = true;
    if (pathname !== "/") void navigate({ to: "/", replace: true });
  }, [navigate, pathname]);

  // When the viewed workstream is archived, fall back to its project view (only
  // when it was the selected item — leaves background archives alone).
  const dismissToProject = useCallback(
    (projectPath: string, id: string) => {
      if (currentProjectPath === projectPath && activeWorkstreamId === id) {
        void navigate({ to: "/projects/$projectPath", params: { projectPath }, search: {} });
      }
    },
    [currentProjectPath, activeWorkstreamId, navigate],
  );

  // Archive-on-merge: when the active workstream's PR is observed merged and the
  // setting is on, archive it automatically. Force-discard since the work has
  // landed remotely (the local branch reads as "unpushed" vs the un-pulled
  // base, so a Safe archive would always refuse). Guarded so it fires once.
  const archiveOnMerge = bootstrap.data?.settings.git?.archiveOnMerge ?? false;
  const autoArchive = useArchiveWorkstream();
  const autoArchivedRef = useRef<Set<string>>(new Set());
  // Enabled for any active workstream (not just archive-on-merge): the Ship
  // button gates on this too. Shares the query cache with PrStatusBar's
  // observer, so it adds no extra `gh` calls.
  const activePr = useWorkstreamPr(activeWorkstreamId ?? "", Boolean(activeWorkstream));
  useEffect(() => {
    if (!archiveOnMerge || !activeWorkstream) return;
    if (activePr.data?.state !== "merged") return;
    const id = activeWorkstream.item.id;
    if (autoArchivedRef.current.has(id)) return;
    autoArchivedRef.current.add(id);
    const projectPath = activeWorkstream.item.projectPath;
    autoArchive.mutate(
      { id, discard: true },
      { onSuccess: () => dismissToProject(projectPath, id) },
    );
  }, [archiveOnMerge, activeWorkstream, activePr.data?.state, autoArchive, dismissToProject]);

  const [itemMenu, setItemMenu] = useState<{
    x: number;
    y: number;
    view: WorkstreamView;
  } | null>(null);
  const [archiveTarget, setArchiveTarget] = useState<WorkstreamView | null>(null);
  const [renameTarget, setRenameTarget] = useState<WorkstreamView | null>(null);
  const [addProjectOpen, setAddProjectOpen] = useState(false);
  const [projectMenu, setProjectMenu] = useState<{ x: number; y: number; project: Project } | null>(
    null,
  );
  const [settingsProject, setSettingsProject] = useState<Project | null>(null);
  const diffHidden = useUiStore((state) => state.diffHidden);
  const toggleDiff = useUiStore((state) => state.toggleDiff);
  const diffWidth = useUiStore((state) => state.diffWidth);

  const contentOffset = Math.max(
    collapsed ? PROJECT_SIDEBAR_COLLAPSED : sidebarWidth,
    IS_MACOS ? TOP_CHROME_MIN_CONTENT_OFFSET : 0,
  );
  // Inset the agent-pane controls (file opener + changes toggle + docs) so they
  // sit at the agent pane's right margin, mirroring how the breadcrumb tracks
  // the left sidebar. Only the agent view has the diff sidebar, and only while
  // it's shown; everywhere else the controls stay flush to the window edge.
  const rightOffset = currentSection === "Agent" && !diffHidden ? diffWidth : 0;
  // The Ship/PR/Archive cluster always pins to the window's right edge,
  // independent of the pane icons. While the diff sidebar is open it floats
  // above the sidebar (absolutely positioned, to the RIGHT of the inset pane
  // icons); otherwise it flows inline just to the LEFT of those icons.
  const shipFloating = rightOffset > 0;
  // Collapse the labelled header controls (the "Ship" button) to icon-only when
  // the toolbar gets crowded — measured off the header's content row so it
  // accounts for the sidebar width, not just the window.
  const headerRef = useRef<HTMLDivElement>(null);
  const headerWidth = useElementWidth(headerRef);
  const compactHeader = headerWidth > 0 && headerWidth < 680;
  const startSidebarResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (collapsed) return;
    startEdgeDrag(event, {
      startWidth: sidebarWidth,
      min: PROJECT_SIDEBAR_MIN,
      max: PROJECT_SIDEBAR_MAX,
      direction: 1,
      onResize: setSidebarWidth,
    });
  };

  // Ship / PR / Archive controls. Rendered in exactly one place per layout
  // (inline left of the pane icons when the sidebar is collapsed, or floating
  // at the window's right edge when it's open), so this is a shared fragment.
  // Ship opens the PR, so it's pointless once one is live. Hide it while the
  // branch already has an open or merged PR (the PR bar's Merge/Archive takes
  // over) — otherwise the agent committing mid-turn leaves the branch `ahead`
  // and Ship would flicker back even though there's nothing left to ship.
  const hasLivePr = activePr.data?.state === "open" || activePr.data?.state === "merged";
  // Commit / Ship actions + their enablement, defined once so the toolbar
  // buttons and the keyboard commands (below) share a single source of truth.
  const canCommit = Boolean(activeWorkstream && activeWorkstream.git.dirty > 0);
  const doCommit = () => {
    if (!activeWorkstream) return;
    setCommitting(true);
    void commitWorkstream(activeWorkstream.item.id).finally(() => setCommitting(false));
  };
  const canShip = Boolean(
    activeWorkstream &&
      !hasLivePr &&
      (activeWorkstream.git.dirty > 0 || activeWorkstream.git.ahead > 0),
  );
  const doShip = () => {
    if (!activeWorkstream) return;
    setShipping(true);
    void shipWorkstream(activeWorkstream.item.id).finally(() => setShipping(false));
  };
  const shipControls = currentProject ? (
    <>
      {/* Commit checkpoints the worktree locally, independent of opening a PR —
          shown whenever there are uncommitted changes, even alongside a live PR. */}
      {canCommit && (
        <button
          type="button"
          disabled={committing}
          onClick={doCommit}
          className="inline-flex h-7 items-center gap-2 rounded-md border border-commit/50 bg-commit/15 px-3 text-sm font-medium text-commit hover:bg-commit/25 disabled:opacity-60"
          title="Commit this worktree's changes (no push or pull request)"
          aria-label="Commit"
        >
          <GitCommitIcon />
          {!compactHeader && "Commit"}
        </button>
      )}
      {canShip && (
        <button
          type="button"
          disabled={shipping}
          onClick={doShip}
          className="inline-flex h-7 items-center gap-2 rounded-md border border-ship/50 bg-ship/15 px-3 text-sm font-medium text-ship hover:bg-ship/25 disabled:opacity-60"
          title="Commit, push, and open a pull request"
          aria-label="Ship"
        >
          <GitPullRequestIcon />
          {!compactHeader && "Ship"}
        </button>
      )}
      <PrStatusBar view={activeWorkstream ?? null} onArchive={setArchiveTarget} />
    </>
  ) : null;

  // Global keyboard shortcuts. The dispatcher reads the user's overrides from
  // bootstrap; each command below registers its handler only while it's usable
  // in the current context (a null handler = "not available now"), so the chord
  // is simply inert elsewhere. Tab/session commands register from ProjectPage.
  useKeybindings(bootstrap.data?.settings.keybindings?.overrides ?? EMPTY_OVERRIDES);
  useCommand("nav.dashboard", () => void navigate({ to: "/" }));
  useCommand("nav.library", () => toggleNav("/library"));
  useCommand("nav.settings", () => toggleNav("/settings"));
  useCommand(
    "nav.projectDocs",
    currentProject ? () => toggleProjectDocs(currentProject.path) : null,
  );
  useCommand("nav.back", canGoBack ? () => router.history.back() : null);
  useCommand("view.toggleProjectSidebar", () => setCollapsed((value) => !value));
  useCommand(
    "view.toggleChangesSidebar",
    currentSection === "Agent" ? () => toggleDiff() : null,
  );
  useCommand("ws.commit", canCommit && !committing ? doCommit : null);
  useCommand("ws.ship", canShip && !shipping ? doShip : null);

  return (
    <div className="flex h-full flex-col bg-surface">
      {/* A tall toolbar with the breadcrumb + window controls vertically
          centered, leaving padding above and below. On macOS the traffic lights
          are pushed down to this bar's center via trafficLightPosition
          (tauri.conf.json y ≈ header height / 2) so they line up with the
          centered content. */}
      <header
        className="relative flex h-11 shrink-0 items-center border-b border-edge bg-panel"
        data-tauri-drag-region
      >
        <div
          className={`absolute top-1/2 flex -translate-y-1/2 items-center gap-1 ${
            IS_MACOS ? "left-[92px]" : "left-3"
          }`}
        >
          <button
            type="button"
            onClick={() => setCollapsed((value) => !value)}
            className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-ink-dim hover:bg-edge/60 hover:text-ink"
            title={collapsed ? "Expand project sidebar" : "Collapse project sidebar"}
            aria-label={collapsed ? "Expand project sidebar" : "Collapse project sidebar"}
          >
            <PanelLeftIcon />
          </button>
          <Link
            to="/"
            className={`inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md hover:bg-edge/60 hover:text-ink ${
              pathname === "/" ? "bg-edge text-ink" : "text-ink-dim"
            }`}
            title="Dashboard"
            aria-label="Dashboard"
          >
            <HomeIcon />
          </Link>
          {/* Project docs lives here (a fixed left anchor) rather than in the
              right cluster, so it doesn't shift when the diff sidebar inset
              changes. */}
          {currentProject && (
            <button
              type="button"
              onClick={() => toggleProjectDocs(currentProject.path)}
              className={`inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md hover:bg-edge/60 hover:text-ink ${
                pathname.startsWith("/projects/") && pathname.endsWith("/docs")
                  ? "bg-edge text-ink"
                  : "text-ink-dim"
              }`}
              title="Project docs"
              aria-label="Project docs"
            >
              <BookIcon size={18} />
            </button>
          )}
        </div>

        <div
          ref={headerRef}
          className="flex min-w-0 flex-1 items-center justify-between gap-4 pl-4 pr-3"
          style={{ marginLeft: contentOffset }}
          data-tauri-drag-region
        >
          <div className="flex min-w-0 items-center gap-2 text-sm" data-tauri-drag-region>
            {currentProject ? (
              <>
                <Link
                  to="/projects/$projectPath"
                  params={{ projectPath: currentProject.path }}
                  className="truncate font-medium text-ink hover:text-ink"
                >
                  {currentProject.mappedName}
                </Link>
                <span className="text-ink-dim">
                  <ChevronRightIcon />
                </span>
                <span className="truncate text-ink-dim">{crumb}</span>
              </>
            ) : (
              <span className="truncate font-medium text-ink">{currentSection}</span>
            )}
          </div>

          <div
            className="flex shrink-0 items-center gap-2"
            style={{ marginRight: rightOffset }}
          >
            {currentProject && (
              <>
                {/* Sidebar collapsed → Ship/PR/Archive flow just left of the
                    pane icons. Sidebar open → they float at the window edge
                    (rendered below), to the right of the inset icons. */}
                {!shipFloating && shipControls}
                <OpenInMenu dir={activeWorkstream?.item.worktreePath ?? currentProject.path} />
                {currentSection === "Agent" && (
                  <button
                    type="button"
                    onClick={toggleDiff}
                    className={topIconClass}
                    title={diffHidden ? "Show changes sidebar" : "Hide changes sidebar"}
                    aria-label={diffHidden ? "Show changes sidebar" : "Hide changes sidebar"}
                  >
                    <PanelRightIcon />
                  </button>
                )}
              </>
            )}
          </div>
        </div>

        {/* Ship/PR/Archive pinned to the window's right edge while the diff
            sidebar is open, so they float above it independently of the
            agent-pane icons (which stay inset to the pane's right margin). */}
        {shipFloating && shipControls && (
          <div className="absolute right-3 top-1/2 flex -translate-y-1/2 items-center gap-2">
            {shipControls}
          </div>
        )}
      </header>

      <div className="flex min-h-0 flex-1">
        <aside
          className="flex shrink-0 flex-col border-r border-edge bg-surface"
          style={{ width: collapsed ? PROJECT_SIDEBAR_COLLAPSED : sidebarWidth }}
        >
          {/* Header band, height-matched to the agent bar (h-9) so the
              "Projects" label + add button sit vertically centered alongside
              it — and the bottom border lines up across the sidebar/pane seam. */}
          <div
            className={`flex h-9 shrink-0 items-center border-b border-edge ${
              collapsed ? "justify-center px-1" : "justify-between pl-4 pr-3"
            }`}
          >
            {!collapsed && (
              <p className="text-xs font-medium uppercase tracking-wider text-ink-dim">Projects</p>
            )}
            <button
              type="button"
              onClick={() => setAddProjectOpen(true)}
              className="flex h-6 w-6 items-center justify-center rounded text-ink-dim hover:bg-edge/60 hover:text-ink"
              title="Add project"
              aria-label="Add project"
            >
              <PlusIcon size={14} />
            </button>
          </div>
          <nav className="flex-1 overflow-y-auto px-2 pt-2">
            {bootstrap.isLoading && (
              <div className="flex justify-center py-4">
                <Spinner />
              </div>
            )}
            {!bootstrap.isLoading && projects.length === 0 && !collapsed && (
              <p className="px-2 py-1 text-sm text-ink-dim">
                {configReady ? "No projects mapped yet." : "Setup required."}
              </p>
            )}
            {projects.map((p) => {
              const children = itemsByProject.get(p.path) ?? [];
              const isCurrent = currentProjectPath === p.path;
              const projectHighlight = isCurrent && !activeWorkstreamId;

              if (collapsed) {
                return (
                  <Link
                    key={p.path}
                    to="/projects/$projectPath"
                    params={{ projectPath: p.path }}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      setItemMenu(null);
                      setProjectMenu({ x: e.clientX, y: e.clientY, project: p });
                    }}
                    className={`mb-1 flex h-9 w-9 items-center justify-center rounded-md hover:bg-edge/60 ${
                      isCurrent ? "bg-edge" : ""
                    }`}
                    title={p.mappedName}
                    aria-label={p.mappedName}
                  >
                    <RepoAvatar
                      source={p.originUrl}
                      name={p.mappedName}
                      size={22}
                      className="rounded-md"
                    />
                  </Link>
                );
              }

              return (
                <div key={p.path} className="mb-0.5">
                  <div
                    onContextMenu={(e) => {
                      e.preventDefault();
                      setItemMenu(null);
                      setProjectMenu({ x: e.clientX, y: e.clientY, project: p });
                    }}
                    className={`group flex items-center rounded-md pr-1 hover:bg-edge/60 ${
                      projectHighlight ? "bg-edge" : ""
                    }`}
                  >
                    <Link
                      to="/projects/$projectPath"
                      params={{ projectPath: p.path }}
                      className={`flex min-w-0 flex-1 items-center gap-1.5 px-2 py-1.5 text-sm hover:text-ink ${
                        projectHighlight ? "text-ink" : "text-ink-dim"
                      }`}
                      title={`${p.mappedName} · ${p.backendKind}${p.profile ? ` · ${p.profile}` : ""}`}
                    >
                      <RepoAvatar source={p.originUrl} name={p.mappedName} size={18} />
                      <span className="truncate">{p.mappedName}</span>
                      {!p.existsOnDisk && (
                        <span
                          className="h-1.5 w-1.5 shrink-0 rounded-full bg-warn"
                          title="Repository folder is missing on disk"
                        />
                      )}
                    </Link>
                    <Link
                      to="/projects/$projectPath"
                      params={{ projectPath: p.path }}
                      search={{ create: true }}
                      className="flex h-6 w-6 shrink-0 items-center justify-center rounded text-ink-dim hover:bg-edge/60 hover:text-ink"
                      title={`New workstream in ${p.mappedName}`}
                      aria-label={`New workstream in ${p.mappedName}`}
                    >
                      <PlusIcon size={14} />
                    </Link>
                  </div>

                  {children.map((view) => {
                    const isActiveItem = isCurrent && activeWorkstreamId === view.item.id;
                    return (
                      <Link
                        key={view.item.id}
                        to="/projects/$projectPath"
                        params={{ projectPath: p.path }}
                        search={{ item: view.item.id }}
                        onContextMenu={(e) => {
                          e.preventDefault();
                          setProjectMenu(null);
                          setItemMenu({ x: e.clientX, y: e.clientY, view });
                        }}
                        className={`flex items-center gap-2 rounded-md py-1.5 pl-6 pr-2 text-sm hover:bg-edge/60 hover:text-ink ${
                          isActiveItem ? "bg-edge text-ink" : "text-ink-dim"
                        }`}
                        title={`${view.item.title} · ${view.item.branch}`}
                      >
                        <WorkstreamActivityDot workstreamId={view.item.id} />
                        <GitBranchIcon />
                        <span className="min-w-0 flex-1 truncate">{view.item.title}</span>
                        <WorkstreamRowChip view={view} />
                      </Link>
                    );
                  })}
                </div>
              );
            })}

          </nav>

        <div className="flex flex-col gap-2 border-t border-edge p-2">
          <UpdatePill collapsed={collapsed} />
          <div
            className={
              collapsed ? "flex flex-col items-center gap-1" : "flex items-center justify-between"
            }
          >
            <button
              type="button"
              onClick={() => toggleNav("/library")}
              className={footerIconClass(pathname.startsWith("/library"))}
              title="Library"
              aria-label="Library"
            >
              <BookIcon size={18} />
            </button>
            <button
              type="button"
              onClick={() => toggleNav("/settings")}
              className={footerIconClass(pathname.startsWith("/settings"))}
              title="Settings"
              aria-label="Settings"
            >
              <SettingsIcon />
            </button>
          </div>
        </div>
        </aside>

        {!collapsed && (
          <div
            role="separator"
            aria-orientation="vertical"
            aria-label="Resize project sidebar"
            className="relative z-10 -mx-1 w-2 shrink-0 cursor-col-resize bg-transparent transition-colors hover:bg-edge/60"
            onPointerDown={startSidebarResize}
          />
        )}

        <main className="min-w-0 flex-1 overflow-y-auto">
          <Outlet />
        </main>
      </div>

      {itemMenu && (
        <ContextMenu position={itemMenu} onClose={() => setItemMenu(null)}>
          <ContextMenuItem
            onClick={() => {
              void navigate({
                to: "/projects/$projectPath",
                params: { projectPath: itemMenu.view.item.projectPath },
                search: { item: itemMenu.view.item.id },
              });
              setItemMenu(null);
            }}
          >
            Open
          </ContextMenuItem>
          <ContextMenuItem
            onClick={() => {
              setRenameTarget(itemMenu.view);
              setItemMenu(null);
            }}
          >
            Rename workstream…
          </ContextMenuItem>
          <ContextMenuItem
            danger
            onClick={() => {
              setArchiveTarget(itemMenu.view);
              setItemMenu(null);
            }}
          >
            Archive workstream…
          </ContextMenuItem>
        </ContextMenu>
      )}

      {projectMenu && (
        <ContextMenu position={projectMenu} onClose={() => setProjectMenu(null)}>
          <ContextMenuItem
            onClick={() => {
              void navigate({
                to: "/projects/$projectPath",
                params: { projectPath: projectMenu.project.path },
                search: {},
              });
              setProjectMenu(null);
            }}
          >
            Open
          </ContextMenuItem>
          <ContextMenuItem
            onClick={() => {
              void navigate({
                to: "/projects/$projectPath",
                params: { projectPath: projectMenu.project.path },
                search: { create: true },
              });
              setProjectMenu(null);
            }}
          >
            New workstream…
          </ContextMenuItem>
          <ContextMenuItem
            onClick={() => {
              setSettingsProject(projectMenu.project);
              setProjectMenu(null);
            }}
          >
            Project settings…
          </ContextMenuItem>
        </ContextMenu>
      )}

      <AddProjectDialog
        open={addProjectOpen}
        onClose={() => setAddProjectOpen(false)}
        onAdded={(project) =>
          void navigate({
            to: "/projects/$projectPath",
            params: { projectPath: project.path },
            search: {},
          })
        }
      />

      {settingsProject && (
        <ProjectSettingsDialog
          projectPath={settingsProject.path}
          onClose={() => setSettingsProject(null)}
        />
      )}

      <RenameWorkstreamDialog target={renameTarget} onClose={() => setRenameTarget(null)} />

      <ArchiveWorkstreamDialog
        target={archiveTarget}
        onClose={() => setArchiveTarget(null)}
        onArchived={(id) => {
          // If the archived item was the one selected, drop back to its project.
          if (archiveTarget) dismissToProject(archiveTarget.item.projectPath, id);
          setArchiveTarget(null);
        }}
      />
    </div>
  );
}
