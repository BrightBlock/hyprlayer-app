import { Link, Outlet, useRouterState } from "@tanstack/react-router";

import { useBootstrap } from "../lib/queries";
import { Badge, Spinner } from "../components/ui";

export function Root() {
  const bootstrap = useBootstrap();
  const pathname = useRouterState({ select: (s) => s.location.pathname });

  const projects = bootstrap.data?.projects ?? [];
  const configReady = bootstrap.data?.config.state === "ready";

  return (
    <div className="flex h-full">
      <aside className="flex w-60 shrink-0 flex-col border-r border-edge bg-panel">
        <div className="flex items-center gap-2 px-4 py-4">
          <span className="text-base font-semibold tracking-tight">Hyprlayer</span>
          <Badge tone="accent">alpha</Badge>
        </div>

        <nav className="flex-1 overflow-y-auto px-2">
          <p className="px-2 pb-1 pt-2 text-xs font-medium uppercase tracking-wider text-ink-dim">
            Projects
          </p>
          {bootstrap.isLoading && (
            <div className="flex justify-center py-4">
              <Spinner />
            </div>
          )}
          {!bootstrap.isLoading && projects.length === 0 && (
            <p className="px-2 py-1 text-sm text-ink-dim">
              {configReady ? "No projects mapped yet." : "Setup required."}
            </p>
          )}
          {projects.map((p) => (
            <Link
              key={p.path}
              to="/projects/$projectPath"
              params={{ projectPath: p.path }}
              className="block rounded-md px-2 py-1.5 text-sm text-ink-dim hover:bg-panel-2 hover:text-ink data-[status=active]:bg-panel-2 data-[status=active]:text-ink"
            >
              <span className="block truncate">{p.mappedName}</span>
              <span className="block truncate text-xs opacity-60">
                {p.backendKind}
                {p.profile ? ` · ${p.profile}` : ""}
                {!p.existsOnDisk ? " · missing" : ""}
              </span>
            </Link>
          ))}

          <p className="px-2 pb-1 pt-4 text-xs font-medium uppercase tracking-wider text-ink-dim">
            Workspace
          </p>
          <Link
            to="/library"
            className="block rounded-md px-2 py-1.5 text-sm text-ink-dim hover:bg-panel-2 hover:text-ink data-[status=active]:bg-panel-2 data-[status=active]:text-ink"
          >
            Library
          </Link>
        </nav>

        <div className="border-t border-edge p-2">
          <Link
            to="/onboarding"
            className="block rounded-md px-2 py-1.5 text-sm text-ink-dim hover:bg-panel-2 hover:text-ink"
          >
            Setup
            {!configReady && bootstrap.data && (
              <span className="ml-2 inline-block h-2 w-2 rounded-full bg-warn align-middle" />
            )}
          </Link>
          <Link
            to="/settings"
            className={`block rounded-md px-2 py-1.5 text-sm hover:bg-panel-2 hover:text-ink ${
              pathname === "/settings" ? "bg-panel-2 text-ink" : "text-ink-dim"
            }`}
          >
            Settings
          </Link>
        </div>
      </aside>

      <main className="min-w-0 flex-1 overflow-y-auto">
        <Outlet />
      </main>
    </div>
  );
}
