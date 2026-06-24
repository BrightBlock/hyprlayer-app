import { lazy } from "react";
import { createRootRoute, createRoute, createRouter } from "@tanstack/react-router";

import { Root } from "./routes/Root";
import { Dashboard } from "./routes/Dashboard";

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

function optionalString(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

/**
 * The Library is a single master–detail view: filters, the selected document,
 * and its body mode all live in the URL so a view is shareable/restorable.
 * `type`/`status`/`scope` hold comma-separated multi-selections; `project` is
 * the scope selector (a thoughts-project key); `doc` is the selected doc's
 * absolute-path id and `mode` its body editing mode.
 */
export type LibrarySearch = {
  type?: string;
  status?: string;
  scope?: string;
  project?: string;
  author?: string;
  tag?: string;
  q?: string;
  doc?: string;
  mode?: "edit" | "preview";
  /** Repo path of the project the user opened the Library from, when they came
   *  via a project's "Docs" affordance — powers the header's back-to-project
   *  button. Absent when the Library is opened standalone (the nav rail). */
  from?: string;
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
  libraryRoute,
  projectRoute,
]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
