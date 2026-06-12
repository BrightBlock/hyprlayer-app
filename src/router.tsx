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
import { LibraryPage } from "./routes/LibraryPage";

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

const libraryRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/library",
  component: LibraryPage,
});

const projectRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/projects/$projectPath",
  component: ProjectPage,
});

const routeTree = rootRoute.addChildren([
  indexRoute,
  onboardingRoute,
  settingsRoute,
  libraryRoute,
  projectRoute,
]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
