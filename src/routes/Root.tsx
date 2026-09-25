import {
  Link,
  Outlet,
  useCanGoBack,
  useNavigate,
  useRouter,
  useRouterState,
} from "@tanstack/react-router";
import {
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react";
import { openPath } from "@tauri-apps/plugin-opener";
import { listen } from "@tauri-apps/api/event";

import {
  useAllWorkstreams,
  useArchiveWorkstream,
  useBootstrap,
  useDetachPr,
  useDoc,
  useDocWebUrl,
  useEditorTargets,
  useReorderProjects,
  useReprovisionWorkstream,
  useWorkstreamPr,
  useWorktreeFilesCheck,
} from "@/lib/queries";
import { errorMessage } from "@/lib/ipc";
import { log } from "@/lib/log";
import { copyDiagnostics } from "@/lib/diagnostics";
import { triggerActiveFileSave } from "@/lib/fileSave";
import { docTitle } from "@/features/library/text";
import { WorkstreamActivityDot, WorkstreamBuildPhase } from "@/features/session/WorkstreamActivity";
import { WorkstreamPlanning } from "@/features/plan/WorkstreamPlanning";
import { startEdgeDrag } from "@/lib/resize";
import {
  isDetachedHead,
  isFolderWorkstream,
  isGitProject,
  isOffBase,
  shortBranch,
} from "@/lib/domain/projects";
import {
  drawerWidth,
  NAV_RAIL_WIDTH,
  projectSidebarWidth,
  PROJECT_SIDEBAR_DEFAULT,
  PROJECT_SIDEBAR_MAX,
  PROJECT_SIDEBAR_MIN,
} from "@/lib/layout";
import { IS_MACOS } from "@/lib/platform";
import { dragRegion } from "@/lib/dragRegion";
import { useIsFullscreen } from "@/lib/useFullscreen";
import { useViewport } from "@/lib/useViewport";
import { useUiStore } from "@/lib/uiStore";
import {
  selectPinnedViews,
  usePinStore,
} from "@/features/workstreams/pinStore";
import { useProjectTreeStore } from "@/features/projects/projectTreeStore";
import { useLibraryScope } from "@/features/library/libraryScope";
import { onboardingDismissed } from "./onboardingState";
import { useCommand, useEscape } from "@/lib/keybindings/registry";
import { useKeybindings } from "@/lib/keybindings/useKeybindings";
import type { Overrides } from "@/lib/keybindings/resolve";
import {
  ContextMenu,
  ContextMenuItem,
  cx,
  MenuItem,
  Popover,
  RAIL_ICON_BASE,
  railNavClass,
  rowClass,
  RowSpine,
  Spinner,
} from "@/components/ui";
import { RepoAvatar } from "@/components/RepoAvatar";
import { HyprlayerMark } from "@/components/Logo";
import { WindowControls } from "@/components/WindowControls";
import { ResizeGrips } from "@/components/ResizeGrips";
import { Icon } from "@/components/Icon";
import { ArchivedWorktreesButton } from "@/features/archive/ArchivedWorktreesButton";
import { closeArchivedWorktrees } from "@/features/archive/archivedWorktreesStore";
import { ArchivedAgentsButton } from "@/features/session/ArchivedAgentsButton";
import { closeArchivedAgents } from "@/features/session/archivedAgentsStore";
import { useMultiplexer } from "@/features/multiplexer/engines";
import { commands, type EditorTarget } from "@/lib/bindings";
import { SearchPalette } from "@/features/search/SearchPalette";
import { toggleSearch } from "@/features/search/searchStore";
import { FullDiskAccessNotice } from "@/features/system/FullDiskAccessNotice";
import { ServerDisconnectedNotice } from "@/features/system/ServerDisconnectedNotice";
import { HostsDialog } from "@/features/hosts/HostsDialog";
import { ServerMenu } from "@/features/hosts/ServerMenu";
import { SettingsButton } from "@/features/settings/SettingsButton";
import { SettingsCrumb, SettingsHeaderMeta } from "@/features/settings/SettingsHeader";
import { parseSettingsSection } from "@/features/settings/sections";
import {
  closeSettingsPopover,
  openSettingsPopover,
  toggleSettingsPopover,
} from "@/features/settings/settingsPopoverStore";
import { ScriptsDockButton } from "@/features/statusbar/ScriptsDockButton";
import { StatusBar } from "@/features/statusbar/StatusBar";
import { ConfirmQuitDialog } from "@/features/system/ConfirmQuitDialog";
import { PromptPalette } from "@/features/prompts/PromptPalette";
import { PromptArgsDialog } from "@/features/prompts/PromptArgsDialog";
import { togglePromptPalette } from "@/features/prompts/promptsStore";
import { usePromptHotkeys } from "@/features/prompts/usePromptHotkeys";
import {
  toggleDock as toggleScriptDock,
  toggleDockMaximized,
  useScriptsStore,
} from "@/features/session/scriptsStore";
import { ArchiveWorkstreamDialog } from "@/features/workstreams/ArchiveWorkstreamDialog";
import { WorkstreamGitChip } from "@/features/workstreams/WorkstreamGitChip";
import { useWorkstreamGitActions } from "@/features/workstreams/useGitActions";
import { AheadBehind } from "@/features/workstreams/AheadBehind";
import { RenameWorkstreamDialog } from "@/features/workstreams/RenameWorkstreamDialog";
import { AttachPrDialog } from "@/features/workstreams/AttachPrDialog";
import { AddProjectDialog } from "@/features/projects/AddProjectDialog";
import { ProjectSettingsDialog } from "@/features/projects/ProjectSettingsDialog";
import {
  closeProjectSettings,
  openProjectSettings,
  useProjectSettingsStore,
} from "@/features/projects/projectSettingsStore";
import { RemoveProjectDialog } from "@/features/projects/RemoveProjectDialog";
import {
  useProjectDrag,
  type ProjectDragHandlers,
  type ProjectDragState,
} from "@/features/projects/useProjectDrag";
import { useUpdate, showUpdateToast } from "@/lib/updater";
import { useToast } from "@/lib/toast";
import { Toast } from "@/components/Toast";
import type { Project, WorkstreamView } from "@/lib/bindings";
import { useLastRoute } from "@/lib/lastRoute";

const EMPTY_OVERRIDES: Overrides = {};

const LEFT_CLUSTER_WITH_LIGHTS = 92;
const LEFT_CLUSTER_NO_LIGHTS = 12;
const LEFT_CLUSTER_ICON = 28;
const LEFT_CLUSTER_GAP = 4;
// macOS app icons pad their art ~10% per side; 20px shows ~16px, matching the md icon tier.
const HEADER_APP_IMAGE = 20;
const HEADER_MARK = 21;

const HEADER_CONTENT_PAD = 16;
const LEFT_CLUSTER_CLEARANCE = 12;

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
  if (pathname.startsWith("/projects/")) return "Agent";
  if (pathname.startsWith("/library")) return "Library";
  if (pathname.startsWith("/prompts")) return "Prompt Library";
  if (pathname.startsWith("/agents")) return "Agents";
  if (pathname.startsWith("/settings")) return "Settings";
  if (pathname.startsWith("/onboarding")) return "Setup";
  if (pathname === "/") return "Home";
  return "Workspace";
}

const topIconClass =
  "inline-flex h-7 w-7 items-center justify-center rounded-md text-ink-dim hover:bg-edge/60 hover:text-ink";

const railHomeClass = `${RAIL_ICON_BASE} bg-accent text-accent-ink shadow-sm`;

function headerAppIcon(app: EditorTarget | undefined): ReactNode {
  if (!app) return <Icon name="code" />;
  if (app.iconDataUrl) return appIcon(app, HEADER_APP_IMAGE);
  if (app.kind === "terminal") return <Icon name="terminal" />;
  if (app.kind === "files") return <Icon name="folder" />;
  return <Icon name="code" />;
}

function appIcon(app: EditorTarget, imageSize = 22): ReactNode {
  if (app.iconDataUrl) {
    return (
      <img
        src={app.iconDataUrl}
        alt=""
        className="block shrink-0 rounded-[4px]"
        style={{ width: imageSize, height: imageSize }}
      />
    );
  }
  switch (app.kind) {
    case "files":
      return <Icon name="folder" />;
    case "terminal":
      return <Icon name="terminal" />;
    default:
      return <Icon name="code" />;
  }
}

function OpenInMenu({ dir }: { dir: string }) {
  const [open, setOpen] = useState(false);
  const targets = useEditorTargets();
  const triggerRef = useRef<HTMLButtonElement>(null);

  const defaultApp = useMemo(() => {
    const apps = targets.data ?? [];
    return (
      apps.find((a) => a.kind === "editor") ??
      apps.find((a) => a.id !== "files")
    );
  }, [targets.data]);

  const items = useMemo(() => {
    const apps = targets.data ?? [];
    const ordered = defaultApp
      ? [defaultApp, ...apps.filter((a) => a.id !== defaultApp.id)]
      : apps;
    return [
      ...ordered.map((app) => ({
        key: app.id,
        label: app.name,
        icon: appIcon(app),
        run: (): void => {
          const p = app.openWith ? openPath(dir, app.openWith) : openPath(dir);
          void p.catch((e) => log.error("open-in", `opening in ${app.name} failed`, e));
        },
      })),
      {
        key: "copy-path",
        label: "Copy path",
        icon: <Icon name="copy" />,
        run: (): void => {
          void navigator.clipboard.writeText(dir);
        },
      },
    ];
  }, [targets.data, defaultApp, dir]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
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

  return (
    <div className="flex items-center">
      <button
        ref={triggerRef}
        type="button"
        onClick={() => setOpen((v) => !v)}
        className={cx(topIconClass, "w-auto gap-0.5 pl-1 pr-0.5", open && "bg-edge/60 text-ink")}
        title="Open in editor or terminal"
        aria-label="Open working directory in editor or terminal"
        aria-haspopup="menu"
        aria-expanded={open}
      >
        {headerAppIcon(defaultApp)}
        <Icon name="chevronDown" size="xs" />
      </button>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={triggerRef}
        align="end"
        className="min-w-44"
      >
        {items.map((it, i) => (
          <MenuItem
            key={it.key}
            onClick={() => {
              setOpen(false);
              it.run();
            }}
          >
            <span className="flex w-full items-center gap-2.5">
              <span className="shrink-0">{it.icon}</span>
              <span className="min-w-0 flex-1 truncate">{it.label}</span>
              {i < 9 && (
                <span className="shrink-0 text-meta text-ink-dim/60">
                  {i + 1}
                </span>
              )}
            </span>
          </MenuItem>
        ))}
      </Popover>
    </div>
  );
}

function projectDragStyle(
  drag: ProjectDragState,
  path: string,
  index: number,
): CSSProperties | undefined {
  if (!drag) return undefined;
  if (drag.path === path) return { transform: `translateY(${drag.dy}px)` };
  const { srcIndex, targetIndex, height } = drag;
  let shift = 0;
  if (srcIndex < targetIndex && index > srcIndex && index <= targetIndex)
    shift = -height;
  else if (srcIndex > targetIndex && index >= targetIndex && index < srcIndex)
    shift = height;
  return {
    transform: shift ? `translateY(${shift}px)` : undefined,
    transition: "transform 180ms ease",
  };
}

function ProjectCopyWarning({ projectPath }: { projectPath: string }) {
  const check = useWorktreeFilesCheck(projectPath);
  const report = check.data;
  const problems = report
    ? [...report.missing, ...report.errors.map((e) => e.entry)]
    : [];
  if (problems.length === 0) return null;
  return (
    <span
      className="flex h-6 w-6 shrink-0 items-center justify-center text-warn-ink"
      title={`Won't be copied to new worktrees:\n${problems.join("\n")}\n\nAdd the file to the repo, or fix the path in Project settings.`}
      aria-label={`Copy file issues: ${problems.join(", ")}`}
    >
      <Icon name="warning" size="sm" />
    </span>
  );
}

export function Root() {
  const bootstrap = useBootstrap();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const routeHref = useRouterState({ select: (s) => s.location.href });
  const locationSearch = useRouterState({
    select: (s) => s.location.search,
  }) as {
    item?: string;
    doc?: string;
    from?: string;
    section?: string;
  };
  const collapsed = useUiStore((state) => state.projectSidebarCollapsed);
  const setProjectSidebarCollapsed = useUiStore(
    (state) => state.setProjectSidebarCollapsed,
  );
  const toggleProjectSidebar = useUiStore(
    (state) => state.toggleProjectSidebar,
  );
  const { width, narrow, compact } = useViewport();
  const narrowDrawer = useUiStore((state) => state.narrowDrawer);
  const setNarrowDrawer = useUiStore((state) => state.setNarrowDrawer);
  const prevNarrowRef = useRef(narrow);
  const prevCompactRef = useRef(compact);
  useEffect(() => {
    if (prevNarrowRef.current && !narrow) {
      setNarrowDrawer(null);
    } else if (prevCompactRef.current && !compact) {
      if (useUiStore.getState().narrowDrawer === "project") setNarrowDrawer(null);
    }
    prevNarrowRef.current = narrow;
    prevCompactRef.current = compact;
  }, [narrow, compact, setNarrowDrawer]);
  const [sidebarWidth, setSidebarWidth] = useState(PROJECT_SIDEBAR_DEFAULT);

  const projects = useMemo(
    () => bootstrap.data?.projects ?? [],
    [bootstrap.data?.projects],
  );
  const configReady = bootstrap.data?.config.state === "ready";
  const currentProjectPath = routeProjectPath(pathname);
  const currentProject = currentProjectPath
    ? projects.find((project) => project.path === currentProjectPath)
    : null;
  const currentSection = routeSection(pathname);
  const onLibrary = pathname.startsWith("/library");
  const onPrompts = pathname.startsWith("/prompts");
  const onSettings = pathname.startsWith("/settings");
  const onAgents = pathname.startsWith("/agents");
  const projectDrawerRoutable = !onLibrary && !onPrompts && !onSettings && !onAgents;
  const diffDrawerRoutable = pathname.startsWith("/projects");
  useEffect(() => {
    const open = useUiStore.getState().narrowDrawer;
    if (!open) return;
    const routable = open === "project" ? projectDrawerRoutable : diffDrawerRoutable;
    if (!routable) setNarrowDrawer(null);
  }, [projectDrawerRoutable, diffDrawerRoutable, setNarrowDrawer]);
  const libDoc = useDoc(onLibrary ? (locationSearch.doc ?? "") : "");
  const libDocTitle = libDoc.data ? docTitle(libDoc.data.meta) : undefined;
  const libDocUrl = useDocWebUrl(onLibrary ? (locationSearch.doc ?? "") : "");
  const backProject =
    onLibrary && locationSearch.from
      ? projects.find((p) => p.path === locationSearch.from)
      : undefined;

  const setLastProject = useLibraryScope((s) => s.setLastProject);
  const lastProject = useLibraryScope((s) => s.lastProject);
  const currentThoughtsProject = currentProject?.thoughtsProject;
  useEffect(() => {
    if (currentThoughtsProject) setLastProject(currentThoughtsProject);
  }, [currentThoughtsProject, setLastProject]);
  const defaultLibraryProject = projects.find(
    (p) => p.thoughtsProject === lastProject,
  )?.thoughtsProject;

  const allWorkstreams = useAllWorkstreams(projects.length > 0);
  const itemsByProject = new Map<string, WorkstreamView[]>();
  for (const view of allWorkstreams.data ?? []) {
    const group = itemsByProject.get(view.item.projectPath) ?? [];
    group.push(view);
    itemsByProject.set(view.item.projectPath, group);
  }

  const reorderProjects = useReorderProjects();
  const [dragOrder, setDragOrder] = useState<string[] | null>(null);
  const projectDragRef = useRef<ProjectDragHandlers>({ commit: () => {} });
  const {
    drag: projectDrag,
    suppressClickRef: projectDragSuppress,
    startDrag: startProjectDrag,
  } = useProjectDrag(projectDragRef);

  const orderedProjects = useMemo(() => {
    if (!dragOrder) return projects;
    const byPath = new Map(projects.map((p) => [p.path, p]));
    const out = dragOrder
      .map((path) => byPath.get(path))
      .filter((p): p is Project => p !== undefined);
    for (const p of projects) if (!dragOrder.includes(p.path)) out.push(p);
    return out;
  }, [projects, dragOrder]);

  projectDragRef.current = {
    commit: (from, to) => {
      const paths = projects.map((p) => p.path);
      const next = [...paths];
      const [moved] = next.splice(from, 1);
      next.splice(Math.max(0, Math.min(to, next.length)), 0, moved);
      if (next.every((p, i) => p === paths[i])) return;
      setDragOrder(next);
      reorderProjects.mutate(next, { onSettled: () => setDragOrder(null) });
    },
  };
  const pinnedIds = usePinStore((state) => state.pinnedIds);
  const pinnedSectionCollapsed = usePinStore((state) => state.collapsed);
  const togglePin = usePinStore((state) => state.togglePin);
  const collapsedProjectPaths = useProjectTreeStore((state) => state.collapsedPaths);
  const toggleProjectCollapsed = useProjectTreeStore((state) => state.toggleCollapsed);
  const togglePinnedSection = usePinStore((state) => state.toggleCollapsed);
  const pinnedViews = selectPinnedViews(pinnedIds, allWorkstreams.data ?? []);
  const projectByPath = new Map(
    projects.map((project) => [project.path, project]),
  );
  const activeWorkstreamId = currentProjectPath
    ? locationSearch.item
    : undefined;
  const activeWorkstream = activeWorkstreamId
    ? (allWorkstreams.data ?? []).find(
        (view) => view.item.id === activeWorkstreamId,
      )
    : undefined;
  const onBoard = currentSection === "Agent" && !locationSearch.item;
  const crumb =
    currentSection === "Agent"
      ? onBoard
        ? "Board"
        : (activeWorkstream?.item.title ?? "Workstreams")
      : currentSection;

  const router = useRouter();
  const canGoBack = useCanGoBack();

  const navigate = useNavigate();

  useEffect(() => {
    if (
      bootstrap.data?.config.state === "missing" &&
      !onboardingDismissed &&
      !pathname.startsWith("/onboarding")
    ) {
      void navigate({ to: "/onboarding" });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bootstrap.data?.config.state, pathname]);

  const closeOrBack = (fallback: () => void) => {
    if (canGoBack) router.history.back();
    else fallback();
  };
  const toggleNav = (
    target: "/library" | "/prompts" | "/agents",
  ) => {
    if (pathname.startsWith(target)) {
      closeOrBack(() => void navigate({ to: "/" }));
    } else if (target === "/library") {
      const lastSearch = useLibraryScope.getState().lastSearch;
      void navigate({
        to: "/library",
        search:
          lastSearch ??
          (defaultLibraryProject ? { project: defaultLibraryProject } : {}),
      });
    } else {
      void navigate({ to: target });
    }
  };
  const openProjectDocs = (project: Project) => {
    const saved =
      useLibraryScope.getState().lastSearchByProject[project.thoughtsProject];
    void navigate({
      to: "/library",
      search: {
        ...saved,
        project: project.thoughtsProject,
        from: project.path,
      },
    });
  };

  const rememberRoute = useLastRoute((s) => s.remember);
  useEffect(() => {
    if (!pathname.startsWith("/onboarding")) rememberRoute(routeHref);
  }, [routeHref, pathname, rememberRoute]);

  const dismissToProject = useCallback(
    (projectPath: string, id: string) => {
      if (currentProjectPath === projectPath && activeWorkstreamId === id) {
        void navigate({
          to: "/projects/$projectPath",
          params: { projectPath },
          search: {},
        });
      }
    },
    [currentProjectPath, activeWorkstreamId, navigate],
  );

  const archiveOnMerge = bootstrap.data?.settings.git?.archiveOnMerge ?? false;
  const autoArchive = useArchiveWorkstream();
  const autoArchivedRef = useRef<Set<string>>(new Set());
  const activePr = useWorkstreamPr(
    activeWorkstreamId ?? "",
    Boolean(activeWorkstream && !isFolderWorkstream(activeWorkstream)),
  );
  useEffect(() => {
    if (!archiveOnMerge || !activeWorkstream) return;
    if (activePr.data?.state !== "merged") return;
    const id = activeWorkstream.item.id;
    if (autoArchivedRef.current.has(id)) return;
    autoArchivedRef.current.add(id);
    const projectPath = activeWorkstream.item.projectPath;
    autoArchive.mutate(
      { id },
      { onSuccess: () => dismissToProject(projectPath, id) },
    );
  }, [
    archiveOnMerge,
    activeWorkstream,
    activePr.data?.state,
    autoArchive,
    dismissToProject,
  ]);

  const [itemMenu, setItemMenu] = useState<{
    x: number;
    y: number;
    view: WorkstreamView;
  } | null>(null);
  const [archiveTarget, setArchiveTarget] = useState<WorkstreamView | null>(
    null,
  );
  const [renameTarget, setRenameTarget] = useState<WorkstreamView | null>(null);
  const [attachPrTarget, setAttachPrTarget] = useState<WorkstreamView | null>(
    null,
  );
  const detachPr = useDetachPr();
  const reprovision = useReprovisionWorkstream();
  const [addProjectOpen, setAddProjectOpen] = useState(false);
  const [projectMenu, setProjectMenu] = useState<{
    x: number;
    y: number;
    project: Project;
  } | null>(null);
  const settingsPath = useProjectSettingsStore((s) => s.path);
  const scriptsActiveWs = useScriptsStore((s) => s.activeWorkstream);
  const scriptsDockOpen = useScriptsStore((s) =>
    s.activeWorkstream ? !!s.dock[s.activeWorkstream]?.open : false,
  );
  const scriptsRunning = useScriptsStore(
    (s) => Object.keys(s.running).length > 0,
  );
  const scriptsPosition = useScriptsStore((s) => s.dockPosition);
  const [removeTarget, setRemoveTarget] = useState<Project | null>(null);
  const diffHidden = useUiStore((state) => state.diffHidden);
  const toggleDiff = useUiStore((state) => state.toggleDiff);
  const diffShown = narrow ? narrowDrawer === "diff" : !diffHidden;
  const toggleDiffOrDrawer = () => {
    if (narrow) setNarrowDrawer(narrowDrawer === "diff" ? null : "diff");
    else toggleDiff();
  };
  const mux = useMultiplexer();
  const toggleScriptsDock = () => {
    if (!scriptsActiveWs) return;
    const inSidebar = scriptsPosition === "right";
    const visible = scriptsDockOpen && (!inSidebar || diffShown);
    if (visible) {
      toggleScriptDock(scriptsActiveWs);
    } else {
      if (!scriptsDockOpen) toggleScriptDock(scriptsActiveWs);
      if (inSidebar && !diffShown) toggleDiffOrDrawer();
    }
  };
  const maximizeScriptsDock = () => {
    if (scriptsActiveWs) toggleDockMaximized(scriptsActiveWs);
  };
  const libraryCollapsed = useUiStore((state) => state.libraryCollapsed);
  const toggleLibrary = useUiStore((state) => state.toggleLibrary);
  const agentsCollapsed = useUiStore((state) => state.agentsCollapsed);
  const toggleAgents = useUiStore((state) => state.toggleAgents);
  const docRailOpen = useUiStore((state) => state.docRailOpen);
  const toggleDocRail = useUiStore((state) => state.toggleDocRail);

  const projectSidebarShown = compact ? narrowDrawer === "project" : !collapsed;
  const projectDrawerWidth = drawerWidth(PROJECT_SIDEBAR_MIN, width - NAV_RAIL_WIDTH);
  const projectColumnWidth =
    compact || collapsed || !projectDrawerRoutable
      ? 0
      : projectSidebarWidth(sidebarWidth, width);
  const sidebarShown = onLibrary
    ? !libraryCollapsed
    : onAgents
      ? !agentsCollapsed
      : onPrompts || onSettings
        ? true
        : projectSidebarShown;
  const toggleSidebar = () => {
    if (onPrompts || onSettings) return;
    if (onLibrary) toggleLibrary();
    else if (onAgents) toggleAgents();
    else if (compact) setNarrowDrawer(narrowDrawer === "project" ? null : "project");
    else toggleProjectSidebar();
  };

  const isFullscreen = useIsFullscreen();
  const macTrafficLights = IS_MACOS && !isFullscreen;
  const leftClusterLeft = macTrafficLights
    ? LEFT_CLUSTER_WITH_LIGHTS
    : LEFT_CLUSTER_NO_LIGHTS;
  const leftClusterIcons =
    (IS_MACOS ? 0 : 1) + 1 + (currentProject && isGitProject(currentProject) ? 1 : 0);
  const leftClusterRight =
    leftClusterLeft +
    leftClusterIcons * LEFT_CLUSTER_ICON +
    (leftClusterIcons - 1) * LEFT_CLUSTER_GAP;
  const minContentOffset =
    leftClusterRight + LEFT_CLUSTER_CLEARANCE - HEADER_CONTENT_PAD;
  const contentOffset = minContentOffset;
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

  useEffect(() => {
    void commands.watchWorktree(activeWorkstreamId ?? null);
  }, [activeWorkstreamId]);

  const gitActions = useWorkstreamGitActions(activeWorkstream ?? null);

  useKeybindings(
    bootstrap.data?.settings.keybindings?.overrides ?? EMPTY_OVERRIDES,
  );
  useCommand("nav.dashboard", () => void navigate({ to: "/" }));
  useCommand("nav.library", () => toggleNav("/library"));
  useCommand("nav.settings", toggleSettingsPopover);
  useCommand(
    "nav.projectDocs",
    currentProject && isGitProject(currentProject)
      ? () => openProjectDocs(currentProject)
      : null,
  );
  useCommand("nav.back", canGoBack ? () => router.history.back() : null);
  useCommand("view.toggleProjectSidebar", toggleSidebar);
  useCommand("terminal.open", scriptsActiveWs ? toggleScriptsDock : null);
  useCommand(
    "terminal.toggleFullscreen",
    scriptsActiveWs ? maximizeScriptsDock : null,
  );
  useCommand(
    "view.toggleChangesSidebar",
    currentSection === "Agent" ? toggleDiffOrDrawer : null,
  );
  useCommand(
    "ws.commit",
    gitActions.canCommit && !gitActions.committing ? gitActions.doCommit : null,
  );
  useCommand(
    "ws.ship",
    gitActions.canShip && !gitActions.shipping ? gitActions.doShip : null,
  );
  useCommand("search.open", () => {
    closeSettingsPopover();
    closeArchivedWorktrees();
    closeArchivedAgents();
    toggleSearch();
  });
  useCommand("prompts.palette", () => {
    closeSettingsPopover();
    closeArchivedWorktrees();
    closeArchivedAgents();
    togglePromptPalette();
  });
  usePromptHotkeys(activeWorkstream ?? null);
  useEscape(
    narrowDrawer
      ? () => setNarrowDrawer(null)
      : pathname.startsWith("/settings")
        ? () => closeOrBack(() => void navigate({ to: "/" }))
        : null,
  );

  const checkNow = useUpdate((s) => s.checkNow);
  const showToast = useToast((s) => s.show);

  useEffect(() => {
    const unlisten = Promise.all([
      listen("menu:check-for-updates", () => {
        void checkNow().then((result) => {
          if (result === "available") {
            showUpdateToast();
          } else if (result === "uptodate") {
            const v = useUpdate.getState().currentVersion;
            showToast(
              "You're up to date",
              `You're running the latest version of Hyprlayer${v ? ` (v${v})` : ""}.`,
              { tone: "ok" },
            );
          } else {
            showToast(
              "Couldn't check for updates",
              "Check your connection and try again.",
              {
                tone: "bad",
              },
            );
          }
        });
      }),
      // Only fires when the webview did not take ⌘, (useKeybindings prevents the default), so opening is idempotent.
      listen("menu:open-settings", () => {
        openSettingsPopover();
      }),
      listen("menu:save-file", () => {
        triggerActiveFileSave();
      }),
      listen("menu:copy-diagnostics", () => {
        void copyDiagnostics();
      }),
    ]);
    return () => {
      void unlisten.then((unlisteners) => {
        for (const off of unlisteners) off();
      });
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="flex h-full flex-col bg-surface">
      <header
        className="relative flex h-10 shrink-0 items-center border-b border-edge bg-panel"
        {...dragRegion()}
      >
        <div
          className="absolute top-1/2 flex -translate-y-1/2 items-center gap-2"
          style={{ left: leftClusterLeft }}
        >
          {!IS_MACOS && (
            <span
              className="flex h-7 w-7 shrink-0 items-center justify-center"
              {...dragRegion("deep")}
              aria-hidden="true"
            >
              <HyprlayerMark size={HEADER_MARK} />
            </span>
          )}
          <button
            type="button"
            onClick={toggleSidebar}
            className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-ink-dim hover:bg-edge/60 hover:text-ink"
            title={sidebarShown ? "Hide sidebar" : "Show sidebar"}
            aria-label={sidebarShown ? "Hide sidebar" : "Show sidebar"}
          >
            <Icon name="panelLeft" />
          </button>
          {currentProject && isGitProject(currentProject) && (
            <button
              type="button"
              onClick={() => openProjectDocs(currentProject)}
              className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-ink-dim hover:bg-edge/60 hover:text-ink"
              title="Project docs"
              aria-label="Project docs"
            >
              <Icon name="library" />
            </button>
          )}
        </div>

        <div
          className={`flex min-w-0 flex-1 items-center justify-between gap-4 pl-4 ${
            IS_MACOS ? "pr-3" : "pr-1"
          }`}
          style={{ marginLeft: contentOffset }}
          {...dragRegion()}
        >
          <div
            className="flex min-w-0 items-center gap-2 text-body"
            {...dragRegion()}
          >
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
                  <Icon name="chevronRight" />
                </span>
                <span className="truncate text-ink-dim">{crumb}</span>
              </>
            ) : onLibrary ? (
              <>
                {backProject && (
                  <>
                    <button
                      type="button"
                      onClick={() =>
                        void navigate({
                          to: "/projects/$projectPath",
                          params: { projectPath: backProject.path },
                          search: {},
                        })
                      }
                      className="flex shrink-0 items-center gap-0.5 font-medium text-ink-dim hover:text-ink"
                      title={`Back to ${backProject.mappedName}`}
                      aria-label={`Back to ${backProject.mappedName}`}
                    >
                      <Icon name="chevronLeft" />
                      <span className="max-w-40 truncate">
                        {backProject.mappedName}
                      </span>
                    </button>
                    <span className="text-ink-dim">
                      <Icon name="chevronRight" />
                    </span>
                  </>
                )}
                <button
                  type="button"
                  onClick={() =>
                    void navigate({
                      to: "/library",
                      search: (prev) => ({
                        ...prev,
                        doc: undefined,
                      }),
                    })
                  }
                  className="shrink-0 font-medium text-ink hover:text-ink"
                >
                  Library
                </button>
                {libDocTitle && (
                  <>
                    <span className="text-ink-dim">
                      <Icon name="chevronRight" />
                    </span>
                    <span className="truncate text-ink-dim">{libDocTitle}</span>
                  </>
                )}
              </>
            ) : onSettings ? (
              <SettingsCrumb section={parseSettingsSection(locationSearch.section)} />
            ) : (
              <span className="truncate font-medium text-ink">
                {currentSection}
              </span>
            )}
          </div>

          <div className="flex shrink-0 items-center gap-2">
            {onSettings && (
              <SettingsHeaderMeta section={parseSettingsSection(locationSearch.section)} />
            )}
            {onLibrary && libDocUrl.data && (
              <a
                href={libDocUrl.data}
                target="_blank"
                rel="noreferrer"
                className="inline-flex items-center gap-1 text-ui font-medium text-ink-dim hover:text-ink"
              >
                View on GitHub <Icon name="external" size="xs" />
              </a>
            )}
            {currentProject &&
              !diffShown &&
              activeWorkstream &&
              (activeWorkstream.git.ahead > 0 ||
                activeWorkstream.git.behind > 0) && (
                <AheadBehind
                  ahead={activeWorkstream.git.ahead}
                  behind={activeWorkstream.git.behind}
                  className="px-1"
                />
              )}
            <ArchivedWorktreesButton />
            {currentProject && (
              <>
                <OpenInMenu
                  dir={
                    activeWorkstream?.item.worktreePath ?? currentProject.path
                  }
                />
                {currentSection === "Agent" && !onBoard && (
                  <button
                    type="button"
                    onClick={toggleDiffOrDrawer}
                    className={topIconClass}
                    title={
                      diffShown
                        ? "Hide changes sidebar"
                        : "Show changes sidebar"
                    }
                    aria-label={
                      diffShown
                        ? "Hide changes sidebar"
                        : "Show changes sidebar"
                    }
                  >
                    <Icon name="panels" />
                  </button>
                )}
              </>
            )}
            {onLibrary && locationSearch.doc && (
              <button
                type="button"
                onClick={toggleDocRail}
                className={topIconClass}
                title={docRailOpen ? "Hide panels" : "Show panels"}
                aria-label={docRailOpen ? "Hide panels" : "Show panels"}
                aria-pressed={docRailOpen}
              >
                <Icon name="panels" />
              </button>
            )}
          </div>
        </div>

        <WindowControls />
      </header>

      <div className="relative flex min-h-0 flex-1" data-shell-row>
        <nav
          className="flex shrink-0 flex-col items-center border-r border-edge bg-surface py-2.5"
          style={{ width: NAV_RAIL_WIDTH }}
          aria-label="Primary"
        >
          <div className="flex flex-col items-center gap-3">
            <Link
              to="/"
              className={railHomeClass}
              title={
                mux.enabled
                  ? "Home · Multiplexer mode on (Settings → Advanced)"
                  : "Home"
              }
              aria-label={mux.enabled ? "Home (Multiplexer mode on)" : "Home"}
            >
              <Icon name={mux.enabled ? "multiplexer" : "home"} size="rail" />
            </Link>
            <button
              type="button"
              onClick={() => {
                if (pathname.startsWith("/projects")) {
                  if (compact) setNarrowDrawer(narrowDrawer === "project" ? null : "project");
                  else toggleProjectSidebar();
                  return;
                }
                const target = currentProjectPath ?? projects[0]?.path;
                if (target) {
                  setProjectSidebarCollapsed(false);
                  if (compact) setNarrowDrawer("project");
                  void navigate({
                    to: "/projects/$projectPath",
                    params: { projectPath: target },
                    search: {},
                  });
                }
              }}
              className={railNavClass(pathname.startsWith("/projects"))}
              title={
                pathname.startsWith("/projects")
                  ? projectSidebarShown
                    ? "Hide projects panel"
                    : "Show projects panel"
                  : "Projects"
              }
              aria-label="Projects"
            >
              <Icon name="projects" size="rail" />
            </button>
            <button
              type="button"
              onClick={() => toggleNav("/library")}
              className={railNavClass(pathname.startsWith("/library"))}
              title="Library"
              aria-label="Library"
            >
              <Icon name="library" size="rail" />
            </button>
            <button
              type="button"
              onClick={() => toggleNav("/prompts")}
              className={railNavClass(pathname.startsWith("/prompts"))}
              title="Prompt library"
              aria-label="Prompt library"
            >
              <Icon name="prompt" size="rail" />
            </button>
            <button
              type="button"
              onClick={() => toggleNav("/agents")}
              className={railNavClass(pathname.startsWith("/agents"))}
              title="Agents"
              aria-label="Agents"
            >
              <Icon name="agents" size="rail" />
            </button>
          </div>
        </nav>
        {narrowDrawer && (
          <div
            role="presentation"
            className="absolute inset-y-0 right-0 z-20"
            style={{ left: NAV_RAIL_WIDTH + projectColumnWidth }}
            onClick={() => setNarrowDrawer(null)}
          />
        )}
        {!onLibrary && !onPrompts && !onSettings && !onAgents && (
          <aside
            className={`shrink-0 overflow-hidden border-r border-edge bg-panel ${
              compact ? "absolute inset-y-0 z-30 shadow-lg" : "relative"
            }`}
            style={
              compact
                ? {
                    left: NAV_RAIL_WIDTH,
                    width: narrowDrawer === "project" ? projectDrawerWidth : 0,
                  }
                : { width: collapsed ? 0 : projectSidebarWidth(sidebarWidth, width) }
            }
            inert={compact ? narrowDrawer !== "project" : collapsed}
          >
            <div
              className="absolute inset-y-0 left-0 flex flex-col"
              style={{
                width: compact
                  ? projectDrawerWidth
                  : projectSidebarWidth(sidebarWidth, width),
              }}
            >
              <div className="flex h-9 shrink-0 items-center justify-between pl-4 pr-3">
                <p className="text-title text-ink-dim">
                  Projects
                </p>
                <button
                  type="button"
                  onClick={() => setAddProjectOpen(true)}
                  className="flex h-6 w-6 items-center justify-center rounded text-ink-dim hover:bg-edge/60 hover:text-ink"
                  title="Add project"
                  aria-label="Add project"
                >
                  <Icon name="add" size="sm" />
                </button>
              </div>
              <nav className="flex-1 overflow-y-auto px-2 pt-2">
                {bootstrap.isLoading && (
                  <div className="flex justify-center py-4">
                    <Spinner />
                  </div>
                )}
                {!bootstrap.isLoading && projects.length === 0 && (
                  <p className="px-2 py-1 text-body text-ink-dim">
                    {configReady ? (
                      "No projects mapped yet."
                    ) : (
                      <Link
                        to="/onboarding"
                        className="underline hover:text-ink"
                      >
                        Setup required.
                      </Link>
                    )}
                  </p>
                )}

                {pinnedViews.length > 0 && (
                  <div className="mb-1 border-b border-edge/60 pb-1.5">
                    <button
                      type="button"
                      onClick={togglePinnedSection}
                      className="flex w-full items-center gap-1.5 rounded-md px-2 py-1 text-label text-ink-dim hover:text-ink"
                      aria-expanded={!pinnedSectionCollapsed}
                      title={
                        pinnedSectionCollapsed
                          ? "Expand pinned"
                          : "Collapse pinned"
                      }
                    >
                      <Icon
                        name="chevronDown"
                        size="xs"
                        className={`transition-transform ${
                          pinnedSectionCollapsed ? "-rotate-90" : ""
                        }`}
                      />
                      <Icon name="pin" size="xs" />
                      <span>Pinned</span>
                    </button>
                    {!pinnedSectionCollapsed &&
                      pinnedViews.map((view) => {
                        const project = projectByPath.get(
                          view.item.projectPath,
                        );
                        const projectName = project?.mappedName ?? "";
                        const isActiveItem =
                          currentProjectPath === view.item.projectPath &&
                          activeWorkstreamId === view.item.id;
                        return (
                          <Link
                            key={`pin-${view.item.id}`}
                            to="/projects/$projectPath"
                            params={{ projectPath: view.item.projectPath }}
                            search={{ item: view.item.id }}
                            onContextMenu={(e) => {
                              e.preventDefault();
                              setProjectMenu(null);
                              setItemMenu({ x: e.clientX, y: e.clientY, view });
                            }}
                            className={`flex items-center gap-2 rounded-lg px-2 py-1.5 text-body hover:bg-edge/50 hover:text-ink ${
                              isActiveItem
                                ? "bg-accent/15 text-ink"
                                : "text-ink-dim"
                            }`}
                            title={`${projectName} · ${view.item.title} · ${view.item.branch}`}
                          >
                            <WorkstreamActivityDot
                              workstreamId={view.item.id}
                            />
                            <RepoAvatar
                              source={project?.originUrl}
                              name={projectName}
                              size={16}
                            />
                            <span className="min-w-0 flex-1 truncate">
                              <span className="text-ink-dim/70">
                                {projectName}
                              </span>
                              <Icon
                                name="chevronRight"
                                size="xs"
                                className="mx-1 inline align-[-1px] text-ink-dim/40"
                              />
                              {view.item.title}
                            </span>
                            <WorkstreamBuildPhase
                              workstreamId={view.item.id}
                              fallback={
                                <WorkstreamPlanning
                                  workstreamId={view.item.id}
                                  fallback={<WorkstreamGitChip view={view} />}
                                />
                              }
                            />
                          </Link>
                        );
                      })}
                  </div>
                )}

                {orderedProjects.map((p, i) => {
                  const children = itemsByProject.get(p.path) ?? [];
                  const isCurrent = currentProjectPath === p.path;
                  const projectHighlight = isCurrent && !activeWorkstreamId;
                  const isProjectCollapsed = collapsedProjectPaths.includes(p.path);
                  const isDragging = projectDrag?.path === p.path;

                  return (
                    <div
                      key={p.path}
                      data-project-path={p.path}
                      style={projectDragStyle(projectDrag, p.path, i)}
                      className={`relative mb-0.5 ${
                        isDragging
                          ? "z-30 rounded-lg bg-panel-2 shadow-2xl ring-1 ring-accent/40"
                          : ""
                      }`}
                    >
                      <div
                        onContextMenu={(e) => {
                          e.preventDefault();
                          setItemMenu(null);
                          setProjectMenu({
                            x: e.clientX,
                            y: e.clientY,
                            project: p,
                          });
                        }}
                        onPointerDown={(e) =>
                          startProjectDrag(e, { path: p.path })
                        }
                        className={`group relative flex select-none items-center overflow-hidden rounded-lg pr-1 ${
                          projectHighlight ? "bg-accent/15" : "hover:bg-edge/50"
                        } ${isDragging ? "cursor-grabbing" : "cursor-grab"}`}
                      >
                        {projectHighlight && (
                          <span
                            className="absolute inset-y-0 left-0 w-[3px] bg-accent"
                            aria-hidden="true"
                          />
                        )}
                        {children.length > 0 ? (
                          <button
                            type="button"
                            data-no-drag
                            onClick={() => toggleProjectCollapsed(p.path)}
                            aria-expanded={!isProjectCollapsed}
                            aria-label={`${isProjectCollapsed ? "Expand" : "Collapse"} ${p.mappedName}`}
                            title={isProjectCollapsed ? "Expand" : "Collapse"}
                            className="ml-1 flex h-5 w-4 shrink-0 items-center justify-center rounded text-ink-dim hover:text-ink"
                          >
                            <Icon
                              name="chevronDown"
                              size="xs"
                              className={`transition-transform ${
                                isProjectCollapsed ? "-rotate-90" : ""
                              }`}
                            />
                          </button>
                        ) : (
                          <span className="ml-1 w-4 shrink-0" aria-hidden="true" />
                        )}
                        <Link
                          to="/projects/$projectPath"
                          params={{ projectPath: p.path }}
                          draggable={false}
                          onClick={(e) => {
                            if (projectDragSuppress.current) e.preventDefault();
                          }}
                          className={`flex min-w-0 flex-1 items-center gap-2 py-2 pl-1.5 pr-2.5 text-body text-ink ${
                            projectHighlight ? "font-semibold" : "font-medium"
                          }`}
                          title={`${p.mappedName} · ${p.backendKind}${p.profile ? ` · ${p.profile}` : ""}`}
                        >
                          <RepoAvatar
                            source={p.originUrl}
                            name={p.mappedName}
                            size={22}
                            className="rounded-md"
                          />
                          <span className="truncate">{p.mappedName}</span>
                          {!p.existsOnDisk && (
                            <span
                              className="h-1.5 w-1.5 shrink-0 rounded-full bg-warn"
                              title="Repository folder is missing on disk"
                            />
                          )}
                        </Link>
                        <span data-no-drag className="contents">
                          <ProjectCopyWarning projectPath={p.path} />
                        </span>
                        {children.length > 0 && (
                          <span className="shrink-0 px-1.5 text-ui tabular-nums text-ink-dim/60 group-hover:hidden">
                            {children.length}
                          </span>
                        )}
                        <Link
                          to="/projects/$projectPath"
                          params={{ projectPath: p.path }}
                          search={{ create: true }}
                          data-no-drag
                          className="hidden h-6 w-6 shrink-0 items-center justify-center rounded text-ink-dim group-hover:flex hover:bg-edge/60 hover:text-ink"
                          title={`New workstream in ${p.mappedName}`}
                          aria-label={`New workstream in ${p.mappedName}`}
                        >
                          <Icon name="add" size="sm" />
                        </Link>
                      </div>

                      {!isProjectCollapsed &&
                        children.map((view) => {
                          const isActiveItem =
                            isCurrent && activeWorkstreamId === view.item.id;
                        const inplace = view.item.worktreeMode === "inplace";
                        const folder = isFolderWorkstream(view);
                        const detached = isDetachedHead(view);
                        const branchedInPlace = inplace && !detached && isOffBase(view);
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
                            className={rowClass({
                              tier: "standard",
                              selected: isActiveItem ? "spine" : "none",
                              interactive: !isActiveItem,
                              className: `flex gap-2.5 overflow-hidden pr-2 text-body ${
                                isActiveItem ? "text-ink" : "text-ink-dim hover:text-ink"
                              }`,
                            })}
                            style={{ paddingLeft: 40 }}
                            title={
                              folder
                                ? `${view.item.title} · project folder`
                                : `${view.item.title} · ${
                                    detached ? "detached HEAD" : view.item.branch
                                  }${
                                    inplace
                                      ? ` · main checkout (base: ${shortBranch(
                                          view.item.baseBranch,
                                        )})`
                                      : ""
                                  }`
                            }
                          >
                            {isActiveItem && <RowSpine />}
                            <WorkstreamActivityDot
                              workstreamId={view.item.id}
                              idle="dot"
                            />
                            <span className="min-w-0 flex-1 truncate">
                              {view.item.title}
                            </span>
                            <span
                              className="flex shrink-0 items-center text-ink-dim/70"
                              title={
                                folder
                                  ? "Runs in the project folder"
                                  : branchedInPlace
                                    ? `Runs in the main checkout, branched onto ${view.item.branch}`
                                    : inplace
                                      ? "Runs in the main checkout (no worktree)"
                                      : "Runs in a dedicated worktree"
                              }
                            >
                              {branchedInPlace ? (
                                <Icon name="worktree" size="xs" className="text-accent" />
                              ) : inplace || folder ? (
                                <Icon name="mainCheckout" size="xs" />
                              ) : (
                                <Icon name="branch" size="xs" />
                              )}
                            </span>
                            <WorkstreamBuildPhase
                              workstreamId={view.item.id}
                              fallback={
                                <WorkstreamPlanning
                                  workstreamId={view.item.id}
                                  fallback={<WorkstreamGitChip view={view} />}
                                />
                              }
                            />
                          </Link>
                        );
                      })}
                    </div>
                  );
                })}
              </nav>
            </div>
          </aside>
        )}

        {!collapsed && !narrow && !onLibrary && !onPrompts && !onSettings && !onAgents && (
          <div
            role="separator"
            aria-orientation="vertical"
            aria-label="Resize project sidebar"
            className="relative z-10 -mx-1 w-2 shrink-0 cursor-col-resize bg-transparent transition-colors hover:bg-edge/60"
            onPointerDown={startSidebarResize}
          />
        )}

        <main className="min-w-0 flex-1 overflow-y-auto">
          <Suspense
            fallback={
              <div className="flex h-full items-center justify-center">
                <Spinner size="lg" />
              </div>
            }
          >
            <Outlet />
          </Suspense>
        </main>
      </div>

      <StatusBar
        workstreamId={activeWorkstreamId ?? null}
        workstream={activeWorkstream ?? null}
        // ProjectPage's resolved workstream: the key its tabs live under.
        archive={scriptsActiveWs ? <ArchivedAgentsButton workstreamId={scriptsActiveWs} /> : null}
        tools={
          scriptsActiveWs && (
            <ScriptsDockButton
              open={scriptsDockOpen}
              running={scriptsRunning}
              onToggle={toggleScriptsDock}
            />
          )
        }
        settings={<SettingsButton />}
        serverMenu={(anchorRef) => <ServerMenu anchorRef={anchorRef} />}
      />

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
              togglePin(itemMenu.view.item.id);
              setItemMenu(null);
            }}
          >
            {pinnedIds.includes(itemMenu.view.item.id)
              ? "Unpin workstream"
              : "Pin workstream"}
          </ContextMenuItem>
          <ContextMenuItem
            onClick={() => {
              setRenameTarget(itemMenu.view);
              setItemMenu(null);
            }}
          >
            Rename workstream…
          </ContextMenuItem>
          {!isFolderWorkstream(itemMenu.view) && (
            <ContextMenuItem
              onClick={() => {
                setAttachPrTarget(itemMenu.view);
                setItemMenu(null);
              }}
            >
              {itemMenu.view.item.prNumber === null ? "Attach PR…" : "Change PR…"}
            </ContextMenuItem>
          )}
          {itemMenu.view.item.prNumber !== null && (
            <ContextMenuItem
              onClick={() => {
                detachPr.mutate(itemMenu.view.item.id);
                setItemMenu(null);
              }}
            >
              Detach PR #{itemMenu.view.item.prNumber}
            </ContextMenuItem>
          )}
          {itemMenu.view.git.exists && itemMenu.view.git.thoughts === "broken" && (
            <ContextMenuItem
              hint="Symlinks into the thoughts repo are missing or stale."
              onClick={() => {
                const { id, title } = itemMenu.view.item;
                setItemMenu(null);
                reprovision.mutate(id, {
                  onSuccess: () => showToast("Thoughts links repaired", title, { tone: "ok" }),
                  onError: (e) =>
                    showToast("Couldn't repair thoughts links", errorMessage(e), { tone: "bad" }),
                });
              }}
            >
              Repair thoughts links
            </ContextMenuItem>
          )}
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
        <ContextMenu
          position={projectMenu}
          onClose={() => setProjectMenu(null)}
        >
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
              openProjectSettings(projectMenu.project.path);
              setProjectMenu(null);
            }}
          >
            Project settings…
          </ContextMenuItem>
          <ContextMenuItem
            danger
            onClick={() => {
              setRemoveTarget(projectMenu.project);
              setProjectMenu(null);
            }}
          >
            Remove project…
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

      {settingsPath && (
        <ProjectSettingsDialog
          projectPath={settingsPath}
          onClose={closeProjectSettings}
        />
      )}

      <RenameWorkstreamDialog
        target={renameTarget}
        onClose={() => setRenameTarget(null)}
      />

      <AttachPrDialog
        target={attachPrTarget}
        onClose={() => setAttachPrTarget(null)}
      />

      <ArchiveWorkstreamDialog
        target={archiveTarget}
        onClose={() => setArchiveTarget(null)}
        onArchived={(id) => {
          if (archiveTarget)
            dismissToProject(archiveTarget.item.projectPath, id);
          setArchiveTarget(null);
        }}
      />

      <RemoveProjectDialog
        target={removeTarget}
        onClose={() => setRemoveTarget(null)}
        onRemoved={(path) => {
          if (currentProjectPath === path) void navigate({ to: "/" });
          setRemoveTarget(null);
        }}
      />

      <SearchPalette />
      <PromptPalette activeWorkstream={activeWorkstream ?? null} />
      <PromptArgsDialog />
      <FullDiskAccessNotice />
      <ServerDisconnectedNotice />
      <HostsDialog />
      <ConfirmQuitDialog />
      {/* After HostsDialog: when the server cannot start, its blocking overlay
          must not cover the update toast, the one way out of a broken build. */}
      <Toast />
      <ResizeGrips />
    </div>
  );
}
