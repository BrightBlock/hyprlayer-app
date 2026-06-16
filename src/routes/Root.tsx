import {
  Link,
  Outlet,
  useCanGoBack,
  useNavigate,
  useRouter,
  useRouterState,
} from "@tanstack/react-router";
import { useState, type PointerEvent as ReactPointerEvent } from "react";

import { useAllWorkstreams, useBootstrap } from "../lib/queries";
import { shipWorkstream } from "../features/session/sessionStore";
import { startEdgeDrag } from "../lib/resize";
import { IS_MACOS } from "../lib/platform";
import { useUiStore } from "../lib/uiStore";
import { ContextMenu, ContextMenuItem, Spinner } from "../components/ui";
import { RepoAvatar } from "../components/RepoAvatar";
import { BookIcon, ChevronRightIcon, PlusIcon } from "../components/icons";
import { ArchiveWorkstreamDialog } from "../features/workstreams/ArchiveWorkstreamDialog";
import { RenameWorkstreamDialog } from "../features/workstreams/RenameWorkstreamDialog";
import { AddProjectDialog } from "../features/projects/AddProjectDialog";
import type { WorkstreamView } from "../lib/bindings";

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

function BookOpenIcon() {
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
      <path d="M12 7v14" />
      <path d="M3 18a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h5a4 4 0 0 1 4 4 4 4 0 0 1 4-4h5a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1h-6a3 3 0 0 0-3 3 3 3 0 0 0-3-3z" />
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

function GitPullRequestIcon() {
  return (
    <svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none">
      <circle cx="6" cy="6" r="2.5" stroke="currentColor" strokeWidth="1.8" />
      <circle cx="18" cy="18" r="2.5" stroke="currentColor" strokeWidth="1.8" />
      <path d="M6 8.5V18a3 3 0 0 0 3 3h1" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
      <path d="M18 15.5V10a4 4 0 0 0-4-4h-2" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
      <path d="m14 3-3 3 3 3" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function ArrowLeftIcon() {
  return (
    <svg aria-hidden="true" width="18" height="18" viewBox="0 0 24 24" fill="none">
      <path d="M19 12H5m0 0 6-6m-6 6 6 6" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
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

function SetupIcon() {
  return (
    <svg aria-hidden="true" width="18" height="18" viewBox="0 0 24 24" fill="none">
      <path d="M12 3v5M12 16v5M5 12h5M14 12h5" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
      <circle cx="12" cy="12" r="2.5" stroke="currentColor" strokeWidth="1.8" />
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
  const prefixes = ["/projects/", "/settings/projects/"];
  const prefix = prefixes.find((item) => pathname.startsWith(item));
  if (!prefix) return null;
  const segment = pathname.slice(prefix.length).split("/")[0];
  return segment ? decodePathSegment(segment) : null;
}

function routeSection(pathname: string): string {
  if (pathname.startsWith("/projects/") && pathname.endsWith("/docs")) return "Docs";
  if (pathname.startsWith("/projects/")) return "Agent";
  if (pathname.startsWith("/settings/projects/")) return "Settings";
  if (pathname.startsWith("/library/doc")) return "Document";
  if (pathname.startsWith("/library")) return "Library";
  if (pathname.startsWith("/settings")) return "Settings";
  if (pathname.startsWith("/onboarding")) return "Setup";
  return "Workspace";
}

const topIconClass =
  "inline-flex h-7 w-7 items-center justify-center rounded-md text-ink-dim hover:bg-edge/60 hover:text-ink";

/** Conductor-style status chip for a sidebar workstream row. */
function WorkstreamRowChip({ view }: { view: WorkstreamView }) {
  if (view.git.dirty > 0) {
    return <span className="shrink-0 text-xs font-medium text-ok">+{view.git.dirty}</span>;
  }
  if (view.git.ahead > 0) {
    return <span className="shrink-0 text-xs font-medium text-accent">↑{view.git.ahead}</span>;
  }
  return null;
}

export function Root() {
  const bootstrap = useBootstrap();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const locationSearch = useRouterState({ select: (s) => s.location.search }) as {
    item?: string;
  };
  const [collapsed, setCollapsed] = useState(false);
  const [sidebarWidth, setSidebarWidth] = useState(240);
  const [shipping, setShipping] = useState(false);

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
  const onSecondaryView =
    pathname.startsWith("/library") ||
    pathname.startsWith("/settings") ||
    pathname.startsWith("/onboarding") ||
    (pathname.startsWith("/projects/") && pathname.endsWith("/docs"));
  const showBack = canGoBack && onSecondaryView;

  // Right-click menu + archive flow for the worktree rows. The view itself
  // carries its project path, so the targets are just the view.
  const navigate = useNavigate();
  const [itemMenu, setItemMenu] = useState<{
    x: number;
    y: number;
    view: WorkstreamView;
  } | null>(null);
  const [archiveTarget, setArchiveTarget] = useState<WorkstreamView | null>(null);
  const [renameTarget, setRenameTarget] = useState<WorkstreamView | null>(null);
  const [addProjectOpen, setAddProjectOpen] = useState(false);
  const diffHidden = useUiStore((state) => state.diffHidden);
  const toggleDiff = useUiStore((state) => state.toggleDiff);

  const contentOffset = Math.max(
    collapsed ? PROJECT_SIDEBAR_COLLAPSED : sidebarWidth,
    IS_MACOS ? TOP_CHROME_MIN_CONTENT_OFFSET : 0,
  );
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

  return (
    <div className="flex h-full flex-col bg-surface">
      {/* A tall toolbar with the breadcrumb + window controls vertically
          centered, leaving padding above and below. On macOS the traffic lights
          are pushed down to this bar's center via trafficLightPosition
          (tauri.conf.json y ≈ header height / 2) so they line up with the
          centered content. */}
      <header
        className="relative flex h-11 shrink-0 items-center border-b border-edge bg-[#101113]"
        data-tauri-drag-region
      >
        <button
          type="button"
          onClick={() => setCollapsed((value) => !value)}
          className={`absolute top-1/2 inline-flex h-7 w-7 shrink-0 -translate-y-1/2 items-center justify-center rounded-md text-ink-dim hover:bg-[#171a20] hover:text-ink ${
            IS_MACOS ? "left-[92px]" : "left-3"
          }`}
          title={collapsed ? "Expand project sidebar" : "Collapse project sidebar"}
          aria-label={collapsed ? "Expand project sidebar" : "Collapse project sidebar"}
        >
          <PanelLeftIcon />
        </button>

        <div
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

          <div className="flex shrink-0 items-center gap-2">
            {showBack && (
              <button
                type="button"
                onClick={() => router.history.back()}
                className="mr-2 inline-flex h-7 w-7 items-center justify-center rounded-md text-ink-dim hover:bg-[#171a20] hover:text-ink"
                title="Back"
                aria-label="Back"
              >
                <ArrowLeftIcon />
              </button>
            )}
            {currentProject && (
              <>
                {activeWorkstream &&
                  (activeWorkstream.git.dirty > 0 || activeWorkstream.git.ahead > 0) && (
                    <button
                      type="button"
                      disabled={shipping}
                      onClick={() => {
                        setShipping(true);
                        void shipWorkstream(activeWorkstream.item.id).finally(() =>
                          setShipping(false),
                        );
                      }}
                      className="inline-flex h-7 items-center gap-2 rounded-md border border-rose-400/40 bg-rose-400/10 px-3 text-sm font-medium text-rose-300 hover:bg-rose-400/20 disabled:opacity-60"
                      title="Commit, push, and open a pull request"
                    >
                      <GitPullRequestIcon />
                      Ship
                    </button>
                  )}
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
                <Link
                  to="/projects/$projectPath/docs"
                  params={{ projectPath: currentProject.path }}
                  className={topIconClass}
                  title="Project docs"
                  aria-label="Project docs"
                >
                  <BookOpenIcon />
                </Link>
                <Link
                  to="/settings/projects/$projectPath"
                  params={{ projectPath: currentProject.path }}
                  className={topIconClass}
                  title="Project settings"
                  aria-label="Project settings"
                >
                  <SettingsIcon size={18} />
                </Link>
              </>
            )}
          </div>
        </div>
      </header>

      <div className="flex min-h-0 flex-1">
        <aside
          className="flex shrink-0 flex-col border-r border-[#30343b] bg-[#08090b]"
          style={{ width: collapsed ? PROJECT_SIDEBAR_COLLAPSED : sidebarWidth }}
        >
          <nav className="flex-1 overflow-y-auto px-2 pt-3">
            {!collapsed ? (
              <div className="flex items-center justify-between pb-1 pl-2 pr-1 pt-2">
                <p className="text-xs font-medium uppercase tracking-wider text-ink-dim">Projects</p>
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
            ) : (
              <button
                type="button"
                onClick={() => setAddProjectOpen(true)}
                className="mb-1 flex h-9 w-9 items-center justify-center rounded-md text-ink-dim hover:bg-[#171a20] hover:text-ink"
                title="Add project"
                aria-label="Add project"
              >
                <PlusIcon size={16} />
              </button>
            )}
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
                    className={`mb-1 flex h-9 w-9 items-center justify-center rounded-md hover:bg-[#171a20] ${
                      isCurrent ? "bg-[#242832]" : ""
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
                    className={`group flex items-center rounded-md pr-1 hover:bg-[#171a20] ${
                      projectHighlight ? "bg-[#242832]" : ""
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
                          setItemMenu({ x: e.clientX, y: e.clientY, view });
                        }}
                        className={`flex items-center gap-2 rounded-md py-1.5 pl-6 pr-2 text-sm hover:bg-[#171a20] hover:text-ink ${
                          isActiveItem ? "bg-[#242832] text-ink" : "text-ink-dim"
                        }`}
                        title={`${view.item.title} · ${view.item.branch}`}
                      >
                        <GitBranchIcon />
                        <span className="min-w-0 flex-1 truncate">{view.item.title}</span>
                        <WorkstreamRowChip view={view} />
                      </Link>
                    );
                  })}
                </div>
              );
            })}

            {!collapsed && (
              <p className="px-2 pb-1 pt-4 text-xs font-medium uppercase tracking-wider text-ink-dim">
                Workspace
              </p>
            )}
            <Link
              to="/library"
              className={
                collapsed
                  ? "mt-3 flex h-9 w-9 items-center justify-center rounded-md text-ink-dim hover:bg-[#171a20] hover:text-ink data-[status=active]:bg-[#242832] data-[status=active]:text-ink"
                  : "block rounded-md px-2 py-1.5 text-sm text-ink-dim hover:bg-[#171a20] hover:text-ink data-[status=active]:bg-[#242832] data-[status=active]:text-ink"
              }
              title="Library"
              aria-label="Library"
            >
              {collapsed ? <BookIcon size={18} /> : "Library"}
            </Link>
          </nav>

        <div className="border-t border-[#181b20] p-2">
          <Link
            to="/onboarding"
            className={
              collapsed
                ? "mb-1 flex h-9 w-9 items-center justify-center rounded-md text-ink-dim hover:bg-[#171a20] hover:text-ink"
                : "block rounded-md px-2 py-1.5 text-sm text-ink-dim hover:bg-[#171a20] hover:text-ink"
            }
            title="Setup"
            aria-label="Setup"
          >
            {collapsed ? (
              <span className="relative">
                <SetupIcon />
                {!configReady && bootstrap.data && (
                  <span className="absolute -right-0.5 -top-0.5 h-2 w-2 rounded-full bg-warn" />
                )}
              </span>
            ) : (
              <>
                Setup
                {!configReady && bootstrap.data && (
                  <span className="ml-2 inline-block h-2 w-2 rounded-full bg-warn align-middle" />
                )}
              </>
            )}
          </Link>
          <Link
            to="/settings"
            className={`${
              collapsed
                ? "flex h-9 w-9 items-center justify-center rounded-md"
                : "block rounded-md px-2 py-1.5 text-sm"
            } hover:bg-[#171a20] hover:text-ink ${
              pathname.startsWith("/settings") ? "bg-[#242832] text-ink" : "text-ink-dim"
            }`}
            title="Settings"
            aria-label="Settings"
          >
            {collapsed ? <SettingsIcon /> : "Settings"}
          </Link>
        </div>
        </aside>

        {!collapsed && (
          <div
            role="separator"
            aria-orientation="vertical"
            aria-label="Resize project sidebar"
            className="-ml-1 w-2 shrink-0 cursor-col-resize bg-transparent transition-colors hover:bg-edge/60"
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

      <RenameWorkstreamDialog target={renameTarget} onClose={() => setRenameTarget(null)} />

      <ArchiveWorkstreamDialog
        target={archiveTarget}
        onClose={() => setArchiveTarget(null)}
        onArchived={(id) => {
          // If the archived item was the one selected, drop back to its project.
          if (
            archiveTarget &&
            currentProjectPath === archiveTarget.item.projectPath &&
            activeWorkstreamId === id
          ) {
            void navigate({
              to: "/projects/$projectPath",
              params: { projectPath: archiveTarget.item.projectPath },
              search: {},
            });
          }
          setArchiveTarget(null);
        }}
      />
    </div>
  );
}
