import { beforeEach, describe, expect, it, vi } from "vitest";

import { queryClient } from "./queryClient";

const listeners = vi.hoisted(() => new Map<string, (event: unknown) => void>());
const stub = vi.hoisted(
  () => (name: string) => ({
    listen: (cb: (event: unknown) => void) => {
      listeners.set(name, cb);
      return Promise.resolve(() => {});
    },
  }),
);

vi.mock("./bindings", () => ({
  commands: {},
  events: {
    configChanged: stub("configChanged"),
    binariesChanged: stub("binariesChanged"),
    omarchyThemeChanged: stub("omarchyThemeChanged"),
    prStatusChanged: stub("prStatusChanged"),
    worktreeChanged: stub("worktreeChanged"),
    workstreamBranchChanged: stub("workstreamBranchChanged"),
    sessionLifecycle: stub("sessionLifecycle"),
    scriptRunChanged: stub("scriptRunChanged"),
    thoughtsIndexUpdated: stub("thoughtsIndexUpdated"),
  },
}));

vi.mock("./theme", () => ({ refreshOmarchyTheme: vi.fn() }));

const { attachEventListeners } = await import("./events");

describe("binariesChanged", () => {
  beforeEach(() => {
    listeners.clear();
    vi.restoreAllMocks();
  });

  it("re-asks everything the hyprlayer binary's identity decides", () => {
    const invalidate = vi.spyOn(queryClient, "invalidateQueries").mockResolvedValue();
    attachEventListeners();
    listeners.get("binariesChanged")?.({});

    const keys = invalidate.mock.calls.map(([arg]) => (arg?.queryKey as string[])?.[0]);
    expect(keys).toEqual(
      expect.arrayContaining(["aiStatus", "agent-bundle", "agent-bundle-versions"]),
    );
    expect(keys).toEqual(expect.arrayContaining(["bootstrap", "health", "project"]));
  });

  it("leaves the filesystem-backed library lists alone", () => {
    const invalidate = vi.spyOn(queryClient, "invalidateQueries").mockResolvedValue();
    attachEventListeners();
    listeners.get("binariesChanged")?.({});

    const keys = invalidate.mock.calls.map(([arg]) => (arg?.queryKey as string[])?.[0]);
    expect(keys).not.toContain("agent-files");
    expect(keys).not.toContain("agent-skills");
  });
});
