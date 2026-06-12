import {
  createRootRoute,
  createRoute,
  createRouter,
  redirect,
} from "@tanstack/react-router";

import { commands } from "./lib/bindings";
import { unwrap } from "./lib/ipc";
import { Root } from "./routes/Root";
import { Onboarding } from "./routes/Onboarding";
import { Settings } from "./routes/Settings";
import { ProjectPage } from "./routes/ProjectPage";
import { ProjectDocsPage } from "./routes/ProjectDocsPage";
import { ProjectSettingsPage } from "./routes/ProjectSettingsPage";
import { LibraryPage } from "./routes/LibraryPage";
import { DocPage } from "./routes/DocPage";

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
  // Land somewhere useful: a ready setup goes to the first project,
  // everything else goes to onboarding.
  beforeLoad: async () => {
    let target: { to: string; params?: Record<string, string> } = { to: "/onboarding" };
    try {
      const bootstrap = await unwrap(commands.getBootstrap());
      const requiredOk = bootstrap.binaries.every((b) => b.optional || b.state.state === "ok");
      const first = bootstrap.projects[0];
      if (requiredOk && bootstrap.config.state === "ready" && first) {
        target = { to: "/projects/$projectPath", params: { projectPath: first.path } };
      }
    } catch {
      // fall through to onboarding
    }
    throw redirect(target as Parameters<typeof redirect>[0]);
  },
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

const projectSettingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings/projects/$projectPath",
  component: ProjectSettingsPage,
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

const projectRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/projects/$projectPath",
  component: ProjectPage,
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
  projectSettingsRoute,
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
