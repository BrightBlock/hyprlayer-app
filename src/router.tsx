import { lazy } from "react";
import { createRootRoute, createRoute, createRouter } from "@tanstack/react-router";

import { Root } from "./routes/Root";
import { Dashboard } from "./routes/Dashboard";
import type { LibrarySearch } from "./features/library/librarySearch";
import { registerAppNav } from "./lib/appNav";
import { useLastRoute } from "./lib/lastRoute";

// The shell (Root) and the landing dashboard load eagerly; everything else is
// code-split so the initial bundle doesn't carry CodeMirror + its language
// grammars (the Library's doc editor / file viewer) or xterm (ProjectPage's
// terminal). Each chunk loads on first navigation, behind Root's Suspense.
const Archive = lazy(() => import("./routes/Archive").then((m) => ({ default: m.Archive })));
const Onboarding = lazy(() =>
  import("./routes/Onboarding").then((m) => ({ default: m.Onboarding })),
);
const Settings = lazy(() => import("./routes/Settings").then((m) => ({ default: m.Settings })));
const ProjectPage = lazy(() =>
  import("./routes/ProjectPage").then((m) => ({ default: m.ProjectPage })),
);
const LibraryPage = lazy(() =>
  import("./routes/LibraryPage").then((m) => ({ default: m.LibraryPage })),
);
const PromptsPage = lazy(() =>
  import("./routes/PromptsPage").then((m) => ({ default: m.PromptsPage })),
);
const AgentsPage = lazy(() =>
  import("./routes/AgentsPage").then((m) => ({ default: m.AgentsPage })),
);

function optionalString(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

const rootRoute = createRootRoute({ component: Root });

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  // The home dashboard is the landing surface. We deliberately do NOT block in
  // beforeLoad on a bootstrap IPC, that delayed first paint. The shell renders
  // instantly. Setup/config lives under Settings now.
  component: Dashboard,
});

const onboardingRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/onboarding",
  component: Onboarding,
});

/** Deep-link a Settings section (e.g. the composer's "Harnesses →" footer). */
export type SettingsSearch = {
  section?: string;
};

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings",
  component: Settings,
  validateSearch: (search: Record<string, unknown>): SettingsSearch => ({
    section: optionalString(search.section),
  }),
});

const archiveRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/archive",
  component: Archive,
});

/** The Prompt library's selection lives in the URL; a bare `/prompts` shows the
 *  default (the built-in Commit prompt). */
export type PromptsSearch = {
  prompt?: string;
};

const promptsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/prompts",
  component: PromptsPage,
  validateSearch: (search: Record<string, unknown>): PromptsSearch => ({
    prompt: optionalString(search.prompt),
  }),
});

/** The agent library's selection lives in the URL; a bare `/agents` is the list.
 *  `section` names the sidebar's root nav choice (agents/system/skills); an
 *  old link without it lands on `agents`, see `AgentsPage`. `project` is the
 *  path of the project the list is scoped to; absent means the global library. */
export type AgentsSearch = {
  agent?: string;
  tab?: string;
  section?: string;
  project?: string;
};

const agentsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/agents",
  component: AgentsPage,
  validateSearch: (search: Record<string, unknown>): AgentsSearch => ({
    agent: optionalString(search.agent),
    tab: optionalString(search.tab),
    section: optionalString(search.section),
    project: optionalString(search.project),
  }),
});

const libraryRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/library",
  component: LibraryPage,
  validateSearch: (search: Record<string, unknown>): LibrarySearch => ({
    type: optionalString(search.type),
    status: optionalString(search.status),
    scope: optionalString(search.scope),
    project: optionalString(search.project),
    author: optionalString(search.author),
    tag: optionalString(search.tag),
    q: optionalString(search.q),
    doc: optionalString(search.doc),
    mode: search.mode === "edit" ? "edit" : undefined,
    from: optionalString(search.from),
  }),
});

/** Work-item selection lives in the URL so the sidebar tree can highlight
 *  and deep-link it; `create` pops the create dialog once, then is stripped. */
export type ProjectSearch = {
  item?: string;
  create?: boolean;
};

const projectRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/projects/$projectPath",
  component: ProjectPage,
  validateSearch: (search: Record<string, unknown>): ProjectSearch => ({
    item: optionalString(search.item),
    create: search.create === true || search.create === "true" || undefined,
  }),
});

const routeTree = rootRoute.addChildren([
  indexRoute,
  onboardingRoute,
  settingsRoute,
  archiveRoute,
  promptsRoute,
  agentsRoute,
  libraryRoute,
  projectRoute,
]);

// Restore the previous session's view by seeding the URL before the router reads
// it at construction, so a cold start mounts straight into the last workstream/
// view instead of flashing the dashboard while a post-mount effect navigates.
// Only when we actually booted to "/", a deep link sets its own URL first. A
// since-deleted project (→ PageError) or a missing config (→ onboarding gate) is
// handled downstream once bootstrap resolves; the route is re-persisted by Root.
const lastHref = useLastRoute.getState().href;
if (lastHref && lastHref !== "/" && window.location.pathname === "/") {
  window.history.replaceState(null, "", lastHref);
}

export const router = createRouter({ routeTree });

// Wire the lower layers' navigation seam to the real router (lib and feature
// modules must not import the router singleton directly).
registerAppNav({
  currentLocation: () => {
    const loc = router.state.location;
    return { pathname: loc.pathname, search: loc.search as Record<string, unknown> };
  },
  goToWorkstream: ({ projectPath, workstreamId }) => {
    void router.navigate({
      to: "/projects/$projectPath",
      params: { projectPath },
      search: { item: workstreamId },
    });
  },
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
