import { lazy } from "react";
import { createRootRoute, createRoute, createRouter, redirect } from "@tanstack/react-router";

import { Root } from "./routes/Root";
import { Dashboard } from "./routes/Dashboard";
import { asLibraryBox } from "./features/library/libraryBoxes";
import { LIBRARY_SORTS, type LibrarySearch } from "./features/library/librarySearch";
import type { WorkshopSearch } from "./features/workshop/workshopSearch";
import { RouteError } from "./components/RouteError";
import { RouteNotFound } from "./components/RouteNotFound";
import { registerAppNav, type ReviewTab } from "./lib/appNav";
import { useLastRoute } from "./lib/lastRoute";
import type { LaunchStage } from "./lib/domain/stages";

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
const WorkshopPage = lazy(() =>
  import("./routes/WorkshopPage").then((m) => ({ default: m.WorkshopPage })),
);

function optionalString(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function optionalLine(value: unknown): number | undefined {
  const n = typeof value === "string" ? Number(value) : value;
  return typeof n === "number" && Number.isInteger(n) && n > 0 ? n : undefined;
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

// Prompts moved into the Workshop; a persisted last route may still say /prompts.
const promptsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/prompts",
  validateSearch: (search: Record<string, unknown>): PromptsSearch => ({
    prompt: optionalString(search.prompt),
  }),
  beforeLoad: ({ search }) => {
    throw redirect({ to: "/workshop", search: { section: "prompts", prompt: search.prompt } });
  },
});

export type AgentsSearch = {
  agent?: string;
  tab?: string;
  section?: string;
  project?: string;
};

const workshopRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/workshop",
  component: WorkshopPage,
  validateSearch: (search: Record<string, unknown>): WorkshopSearch => ({
    section: optionalString(search.section),
    project: optionalString(search.project),
    folder: optionalString(search.folder),
    filter: optionalString(search.filter),
    agent: optionalString(search.agent),
    tab: optionalString(search.tab),
    skill: optionalString(search.skill),
    prompt: optionalString(search.prompt),
  }),
});

// Agents moved into the Workshop; a persisted last route may still say /agents.
const agentsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/agents",
  validateSearch: (search: Record<string, unknown>): AgentsSearch => ({
    agent: optionalString(search.agent),
    tab: optionalString(search.tab),
    section: optionalString(search.section),
    project: optionalString(search.project),
  }),
  beforeLoad: ({ search }) => {
    throw redirect({
      to: "/workshop",
      search: {
        section: search.section,
        project: search.project,
        agent: search.agent,
        tab: search.tab,
      },
    });
  },
});

// The pipeline is the project view now; a persisted last route may still say /pipeline.
const pipelineRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/pipeline",
  beforeLoad: () => {
    throw redirect({ to: "/" });
  },
});

const libraryRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/library",
  component: LibraryPage,
  validateSearch: (search: Record<string, unknown>): LibrarySearch => ({
    box: optionalString(search.box) ? asLibraryBox(optionalString(search.box)) : undefined,
    sort: LIBRARY_SORTS.find((s) => s === search.sort),
    type: optionalString(search.type),
    status: optionalString(search.status),
    scope: optionalString(search.scope),
    project: optionalString(search.project),
    author: optionalString(search.author),
    tag: optionalString(search.tag),
    q: optionalString(search.q),
    doc: optionalString(search.doc),
    from: optionalString(search.from),
  }),
});

export type ProjectSearch = {
  item?: string;
  create?: boolean;
  stage?: LaunchStage;
  review?: ReviewTab;
  reviewFile?: string;
  reviewLine?: number;
};

const projectRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/projects/$projectPath",
  component: ProjectPage,
  validateSearch: (search: Record<string, unknown>): ProjectSearch => ({
    item: optionalString(search.item),
    create: search.create === true || search.create === "true" || undefined,
    stage: search.stage === "research" || search.stage === "plan" ? search.stage : undefined,
    review: search.review === "files" || search.review === "plan" ? search.review : undefined,
    reviewFile: optionalString(search.reviewFile),
    reviewLine: optionalLine(search.reviewLine),
  }),
});

const routeTree = rootRoute.addChildren([
  indexRoute,
  onboardingRoute,
  settingsRoute,
  archiveRoute,
  promptsRoute,
  agentsRoute,
  workshopRoute,
  pipelineRoute,
  libraryRoute,
  projectRoute,
]);

const lastHref = useLastRoute.getState().href;
if (lastHref && lastHref !== "/" && window.location.pathname === "/") {
  window.history.replaceState(null, "", lastHref);
}

// Per-route boundaries: without one, a page's render error replaced the whole
// app, window controls included, with the router's bare fallback.
export const router = createRouter({
  routeTree,
  defaultErrorComponent: RouteError,
  defaultNotFoundComponent: RouteNotFound,
});

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
  openReview: ({ projectPath, workstreamId, tab = "files", file, line }) => {
    void router.navigate({
      to: "/projects/$projectPath",
      params: { projectPath },
      search: { item: workstreamId, review: tab, reviewFile: file, reviewLine: line },
    });
  },
  closeReview: () => {
    void router.navigate({
      from: "/projects/$projectPath",
      to: ".",
      search: (prev) => ({ ...prev, review: undefined, reviewFile: undefined, reviewLine: undefined }),
    });
  },
  openPrompt: (prompt) => {
    void router.navigate({ to: "/workshop", search: { section: "prompts", prompt } });
  },
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
