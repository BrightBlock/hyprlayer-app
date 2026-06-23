import { lazy } from "react";
import { createRootRoute, createRoute, createRouter } from "@tanstack/react-router";

import { Root } from "./routes/Root";
import { Dashboard } from "./routes/Dashboard";

// The shell (Root) and the landing dashboard load eagerly; everything else is
// code-split so the initial bundle doesn't carry CodeMirror + its language
// grammars (DocPage / file viewer) or xterm (ProjectPage's terminal). Each
// chunk loads on first navigation, behind the Suspense boundary in Root.
const Archive = lazy(() => import("./routes/Archive").then((m) => ({ default: m.Archive })));
const Onboarding = lazy(() =>
  import("./routes/Onboarding").then((m) => ({ default: m.Onboarding })),
);
const Settings = lazy(() => import("./routes/Settings").then((m) => ({ default: m.Settings })));
const ProjectPage = lazy(() =>
  import("./routes/ProjectPage").then((m) => ({ default: m.ProjectPage })),
);
const ProjectDocsPage = lazy(() =>
  import("./routes/ProjectDocsPage").then((m) => ({ default: m.ProjectDocsPage })),
);
const LibraryPage = lazy(() =>
  import("./routes/LibraryPage").then((m) => ({ default: m.LibraryPage })),
);
const DocPage = lazy(() => import("./routes/DocPage").then((m) => ({ default: m.DocPage })));

function optionalString(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

/**
 * Library filter state lives in the URL so views are shareable/restorable.
 * `type`/`status`/`scope` hold comma-separated multi-selections.
 */
export type LibrarySearch = {
  type?: string;
  status?: string;
  scope?: string;
  project?: string;
  author?: string;
  tag?: string;
  q?: string;
};

export type ProjectDocsSearch = Omit<LibrarySearch, "project">;

export type DocSearch = {
  /** Absolute-path doc id. */
  id: string;
  mode: "edit" | "preview";
};

const rootRoute = createRootRoute({ component: Root });

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  // The home dashboard is the landing surface. We deliberately do NOT block in
  // beforeLoad on a bootstrap IPC — that delayed first paint. The shell renders
  // instantly. Setup/config lives under Settings now.
  component: Dashboard,
});

const onboardingRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/onboarding",
  component: Onboarding,
});

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings",
  component: Settings,
});

const archiveRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/archive",
  component: Archive,
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
  }),
});

const docRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/library/doc",
  component: DocPage,
  validateSearch: (search: Record<string, unknown>): DocSearch => ({
    id: typeof search.id === "string" ? search.id : "",
    mode: search.mode === "edit" ? "edit" : "preview",
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

const projectDocsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/projects/$projectPath/docs",
  component: ProjectDocsPage,
  validateSearch: (search: Record<string, unknown>): ProjectDocsSearch => ({
    type: optionalString(search.type),
    status: optionalString(search.status),
    scope: optionalString(search.scope),
    author: optionalString(search.author),
    tag: optionalString(search.tag),
    q: optionalString(search.q),
  }),
});

const routeTree = rootRoute.addChildren([
  indexRoute,
  onboardingRoute,
  settingsRoute,
  archiveRoute,
  libraryRoute,
  docRoute,
  projectDocsRoute,
  projectRoute,
]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
