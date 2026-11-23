import { beforeEach, describe, expect, it, vi } from "vitest";

import type { GoalChanged } from "./bindings";
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
    workstreamsChanged: stub("workstreamsChanged"),
    leadQueueChanged: stub("leadQueueChanged"),
    worktreeChanged: stub("worktreeChanged"),
    reviewChanged: stub("reviewChanged"),
    workstreamBranchChanged: stub("workstreamBranchChanged"),
    sessionLifecycle: stub("sessionLifecycle"),
    scriptRunChanged: stub("scriptRunChanged"),
    thoughtsIndexUpdated: stub("thoughtsIndexUpdated"),
    docStateChanged: stub("docStateChanged"),
    serverDisconnected: stub("serverDisconnected"),
    eventsMissed: stub("eventsMissed"),
    accountChanged: stub("accountChanged"),
    goalChanged: stub("goalChanged"),
  },
}));

vi.mock("./theme", () => ({ refreshOmarchyTheme: vi.fn() }));

const { attachEventListeners, onEventsMissed } = await import("./events");
const { onWorkstreamArchived } = await import("./queries");

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

describe("serverDisconnected", () => {
  it("re-reads the level connection state the edge event announces", () => {
    const invalidate = vi.spyOn(queryClient, "invalidateQueries").mockResolvedValue();
    attachEventListeners();
    listeners.get("serverDisconnected")?.({});

    const keys = invalidate.mock.calls.map(([arg]) => (arg?.queryKey as string[])?.[0]);
    expect(keys).toContain("connection-status");
  });
});

describe("eventsMissed", () => {
  beforeEach(() => {
    listeners.clear();
    vi.restoreAllMocks();
  });

  it("re-reads every query and runs the registered re-syncs", () => {
    const invalidate = vi.spyOn(queryClient, "invalidateQueries").mockResolvedValue();
    const resync = vi.fn();
    onEventsMissed(resync);
    attachEventListeners();
    listeners.get("eventsMissed")?.({});

    expect(invalidate).toHaveBeenCalledWith();
    expect(resync).toHaveBeenCalledOnce();
  });
});

describe("workstreamsChanged", () => {
  beforeEach(() => {
    listeners.clear();
    vi.restoreAllMocks();
  });

  it("re-reads the lists and cleans up after a workstream that left its project's list", async () => {
    const key = ["workstreams", "/repo"];
    queryClient.setQueryData(key, [{ item: { id: "ws-kept" } }, { item: { id: "ws-gone" } }]);
    const invalidate = vi.spyOn(queryClient, "invalidateQueries").mockImplementation(async () => {
      queryClient.setQueryData(key, [{ item: { id: "ws-kept" } }, { item: { id: "ws-new" } }]);
    });
    const cleanup = vi.fn();
    onWorkstreamArchived(cleanup);
    attachEventListeners();
    listeners.get("workstreamsChanged")?.({ payload: { projectPath: "/repo" } });

    await vi.waitFor(() => expect(cleanup).toHaveBeenCalledOnce());
    expect(cleanup).toHaveBeenCalledWith("ws-gone");
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["workstreams"] });
  });
});

describe("accountChanged", () => {
  beforeEach(() => {
    listeners.clear();
    vi.restoreAllMocks();
  });

  it("re-reads the account", () => {
    const invalidate = vi.spyOn(queryClient, "invalidateQueries").mockResolvedValue();
    attachEventListeners();
    listeners.get("accountChanged")?.({});
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["account"] });
  });
});

describe("goalChanged", () => {
  const ws = "ws-1";
  const goal = {
    id: "g-1",
    sessionId: "s-1",
    workstreamId: ws,
    status: "running",
    step: "working",
    pauseReason: null,
    iteration: 2,
    limits: { maxIterations: 10 },
    activeMs: 0,
    updatedAt: 1,
  };
  const brief = { id: "g-1", status: "running", step: "working", pauseReason: null, iteration: 2, maxIterations: 10 };
  const payload: GoalChanged = {
    goalId: "g-1",
    sessionId: "s-1",
    workstreamId: ws,
    status: "running",
    step: "checking",
    pauseReason: null,
    endReason: null,
    reason: null,
    iteration: 2,
    maxIterations: 10,
  };

  beforeEach(() => {
    listeners.clear();
    vi.restoreAllMocks();
    queryClient.clear();
    queryClient.setQueryData(["goals", ws, false], [goal]);
    queryClient.setQueryData(["goals", "ws-other", false], []);
    queryClient.setQueryData(["goal", "g-1"], { goal, iterations: [] });
    queryClient.setQueryData(["workstream-sessions", ws], [{ id: "s-1", goal: brief }]);
    queryClient.setQueryData(["active-sessions"], [{ session: { id: "s-1", goal: brief } }]);
  });

  function fire(change: Partial<GoalChanged>) {
    const invalidate = vi.spyOn(queryClient, "invalidateQueries").mockResolvedValue();
    attachEventListeners();
    listeners.get("goalChanged")?.({ payload: { ...payload, ...change } });
    return invalidate.mock.calls.map(([arg]) => arg?.queryKey);
  }

  it("patches a step-only change in place without refetching", () => {
    const keys = fire({});
    expect(keys).toEqual([]);
    expect(queryClient.getQueryData<(typeof goal)[]>(["goals", ws, false])?.[0].step).toBe("checking");
    expect(queryClient.getQueryData<{ goal: typeof goal }>(["goal", "g-1"])?.goal.step).toBe("checking");
    expect(queryClient.getQueryData<{ goal: typeof brief }[]>(["workstream-sessions", ws])?.[0].goal.step).toBe(
      "checking",
    );
  });

  it("re-reads the workstream's goals and the detail when the status changes", () => {
    const keys = fire({ status: "paused", step: null, pauseReason: "user" });
    expect(keys).toEqual(expect.arrayContaining([["goals", ws, false], ["goal", "g-1"]]));
    expect(keys).not.toContainEqual(["goals"]);
    expect(keys).not.toContainEqual(["goals", "ws-other", false]);
    expect(
      queryClient.getQueryData<{ session: { goal: typeof brief } }[]>(["active-sessions"])?.[0].session.goal.status,
    ).toBe("paused");
  });

  it("re-reads the clock when the step enters or leaves waiting", () => {
    const keys = fire({ step: "waiting" });
    expect(keys).toEqual(expect.arrayContaining([["goals", ws, false], ["goal", "g-1"]]));
  });

  it("re-reads a list that doesn't hold the goal yet", () => {
    queryClient.setQueryData(["goals", ws, true], []);
    const keys = fire({});
    expect(keys).toEqual([["goals", ws, true]]);
  });

  it("clears an ended goal's session brief", () => {
    fire({ status: "met", step: null });
    expect(queryClient.getQueryData<{ goal: unknown }[]>(["workstream-sessions", ws])?.[0].goal).toBeNull();
  });
});
