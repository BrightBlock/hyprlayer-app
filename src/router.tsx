import { lazy } from "react";
import { createRootRoute, createRoute, createRouter, redirect } from "@tanstack/react-router";

import { Root } from "./routes/Root";
import { Dashboard } from "./routes/Dashboard";
import type { LibrarySearch } from "./features/library/librarySearch";
import { RouteError } from "./components/RouteError";
import { registerAppNav } from "./lib/appNav";
import { useLastRoute } from "./lib/lastRoute";

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
  component: Dashboard,
});

const onboardingRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/onboarding",
  component: Onboarding,
});

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

// The archive is a header popover now; a persisted last route may still say /archive.
const archiveRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/archive",
  beforeLoad: () => {
    throw redirect({ to: "/" });
  },
});

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

const lastHref = useLastRoute.getState().href;
if (lastHref && lastHref !== "/" && window.location.pathname === "/") {
  window.history.replaceState(null, "", lastHref);
}

// Per-route boundaries: without one, a page's render error replaced the whole
// app, window controls included, with the router's bare fallback.
export const router = createRouter({ routeTree, defaultErrorComponent: RouteError });

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
