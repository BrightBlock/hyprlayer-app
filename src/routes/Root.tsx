import { Link, Outlet, useRouterState } from "@tanstack/react-router";
import { useState, type PointerEvent as ReactPointerEvent } from "react";

import { useBootstrap } from "../lib/queries";
import { Spinner } from "../components/ui";

const PROJECT_SIDEBAR_COLLAPSED = 56;
const PROJECT_SIDEBAR_MIN = 180;
const PROJECT_SIDEBAR_MAX = 360;
const TOP_CHROME_MIN_CONTENT_OFFSET = 148;

function PanelLeftIcon() {
  return (
    <svg aria-hidden="true" width="18" height="18" viewBox="0 0 24 24" fill="none">
      <path
        d="M4 5.5A1.5 1.5 0 0 1 5.5 4h13A1.5 1.5 0 0 1 20 5.5v13a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 4 18.5v-13Z"
        stroke="currentColor"
        strokeWidth="1.8"
      />
      <path d="M10 4v16" stroke="currentColor" strokeWidth="1.8" />
    </svg>
  );
}

function BookOpenIcon() {
  return (
    <svg aria-hidden="true" width="17" height="17" viewBox="0 0 24 24" fill="none">
      <path
        d="M4 5.5A2.5 2.5 0 0 1 6.5 3H11v16H6.5A2.5 2.5 0 0 0 4 21.5v-16Z"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinejoin="round"
      />
      <path
        d="M20 5.5A2.5 2.5 0 0 0 17.5 3H13v16h4.5a2.5 2.5 0 0 1 2.5 2.5v-16Z"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function ChevronRightIcon() {
  return (
    <svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none">
      <path d="m9 6 6 6-6 6" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
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

function LibraryIcon() {
  return (
    <svg aria-hidden="true" width="18" height="18" viewBox="0 0 24 24" fill="none">
      <path d="M4 19.5V5a2 2 0 0 1 2-2h12v18H6a2 2 0 0 1-2-1.5Z" stroke="currentColor" strokeWidth="1.8" />
      <path d="M8 7h6M8 11h7" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
    </svg>
  );
}

function SettingsIcon() {
  return (
    <svg aria-hidden="true" width="18" height="18" viewBox="0 0 24 24" fill="none">
      <path d="M12 9a3 3 0 1 1 0 6 3 3 0 0 1 0-6Z" stroke="currentColor" strokeWidth="1.8" />
      <path
        d="m18 13 .8.6-1.5 2.6-1-.4a6 6 0 0 1-1.2.7l-.2 1.1h-3l-.2-1.1a6 6 0 0 1-1.2-.7l-1 .4-1.5-2.6.8-.6a6 6 0 0 1 0-1.8l-.8-.6L9.5 8l1 .4a6 6 0 0 1 1.2-.7l.2-1.1h3l.2 1.1a6 6 0 0 1 1.2.7l1-.4 1.5 2.6-.8.6a6 6 0 0 1 0 1.8Z"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinejoin="round"
      />
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

function projectInitial(name: string): string {
  return name.trim().slice(0, 1).toUpperCase() || "?";
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
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
  "inline-flex h-8 w-8 items-center justify-center rounded-md border border-edge bg-panel-2 text-ink-dim hover:bg-edge/60 hover:text-ink";

export function Root() {
  const bootstrap = useBootstrap();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const [collapsed, setCollapsed] = useState(false);
  const [sidebarWidth, setSidebarWidth] = useState(240);

  const projects = bootstrap.data?.projects ?? [];
  const configReady = bootstrap.data?.config.state === "ready";
  const currentProjectPath = routeProjectPath(pathname);
  const currentProject = currentProjectPath
    ? projects.find((project) => project.path === currentProjectPath)
    : null;
  const currentSection = routeSection(pathname);
  const contentOffset = Math.max(
    collapsed ? PROJECT_SIDEBAR_COLLAPSED : sidebarWidth,
    TOP_CHROME_MIN_CONTENT_OFFSET,
  );
  const startSidebarResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (collapsed) return;

    event.preventDefault();
    const startX = event.clientX;
    const startWidth = sidebarWidth;
    const previousCursor = document.body.style.cursor;
    const previousSelect = document.body.style.userSelect;

    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";

    const onPointerMove = (moveEvent: PointerEvent) => {
      setSidebarWidth(
        clamp(
          startWidth + moveEvent.clientX - startX,
          PROJECT_SIDEBAR_MIN,
          PROJECT_SIDEBAR_MAX,
        ),
      );
    };
    const onPointerUp = () => {
      document.body.style.cursor = previousCursor;
      document.body.style.userSelect = previousSelect;
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", onPointerUp);
    };

    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp, { once: true });
  };

  return (
    <div className="flex h-full flex-col bg-surface">
      <header
        className="relative flex h-14 shrink-0 items-center border-b border-edge bg-[#101113]"
        data-tauri-drag-region
      >
        <button
          type="button"
          onClick={() => setCollapsed((value) => !value)}
          className="absolute left-[92px] top-3 inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-ink-dim hover:bg-[#171a20] hover:text-ink"
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
                <span className="truncate text-ink-dim">{currentSection}</span>
              </>
            ) : (
              <span className="truncate font-medium text-ink">{currentSection}</span>
            )}
          </div>

          <div className="flex shrink-0 items-center gap-2">
            {currentProject && (
              <>
                <button
                  type="button"
                  className="inline-flex h-8 items-center gap-2 rounded-md border border-edge bg-panel-2 px-3 text-sm font-medium text-ink hover:bg-edge/60"
                  title="Create PR workflow is not connected yet"
                >
                  <GitPullRequestIcon />
                  Create PR
                </button>
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
                  <SettingsIcon />
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
          <nav className={`flex-1 overflow-y-auto pt-3 ${collapsed ? "px-2" : "px-2"}`}>
            {!collapsed && (
              <p className="px-2 pb-1 pt-2 text-xs font-medium uppercase tracking-wider text-ink-dim">
                Projects
              </p>
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
            {projects.map((p) => (
              <Link
                key={p.path}
                to="/projects/$projectPath"
                params={{ projectPath: p.path }}
                className={
                  collapsed
                    ? "mb-1 flex h-9 w-9 items-center justify-center rounded-md text-sm font-semibold text-ink-dim hover:bg-[#171a20] hover:text-ink data-[status=active]:bg-[#242832] data-[status=active]:text-ink"
                    : "block rounded-md px-2 py-1.5 text-sm text-ink-dim hover:bg-[#171a20] hover:text-ink data-[status=active]:bg-[#242832] data-[status=active]:text-ink"
                }
                title={p.mappedName}
                aria-label={p.mappedName}
              >
                {collapsed ? (
                  projectInitial(p.mappedName)
                ) : (
                  <>
                    <span className="block truncate">{p.mappedName}</span>
                    <span className="block truncate text-xs opacity-60">
                      {p.backendKind}
                      {p.profile ? ` · ${p.profile}` : ""}
                      {!p.existsOnDisk ? " · missing" : ""}
                    </span>
                  </>
                )}
              </Link>
            ))}

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
              {collapsed ? <LibraryIcon /> : "Library"}
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
    </div>
  );
}
