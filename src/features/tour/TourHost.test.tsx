// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { ESCAPE_COMMAND_ID, runCommand } from "@/lib/keybindings/registry";
import { pathSteps } from "./selection";
import { firstUnmet, hasEngine, routeProject, TourHost, tourProject } from "./TourHost";
import { activeTour, nextStep, pausedTour, setTaskSentAt, startTour, useTourStore } from "./tourStore";
import { ANCHOR_WAIT_MS } from "./useAnchor";

const navigate = vi.fn();
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => navigate,
  useRouterState: ({ select }: { select: (s: unknown) => unknown }) =>
    select({ location: { pathname: "/projects/%2Frepo", search: {} } }),
}));

const facts = vi.hoisted(() => ({
  lead: true,
  connected: true,
  engine: true,
  projects: [{ path: "/repo" }],
  archived: [] as { item: { id: string; projectPath: string; createdAt: number; parentWorkstreamId: string } }[],
  active: [] as { session: { id: string; workstreamId: string; parentSessionId: string; startedAt: number } }[],
}));

vi.mock("@/lib/queries", () => ({
  useBootstrap: () => ({
    data: {
      projects: facts.projects,
      binaries: [{ kind: "claude", state: { state: facts.engine ? "ok" : "missing" } }],
      settings: { keybindings: { overrides: {} } },
    },
  }),
  useConnectionStatus: () => ({ data: { state: facts.connected ? "connected" : "lost" } }),
  useWorkstreams: () => ({ data: [] }),
  useArchivedWorkstreams: (enabled: boolean) => ({ data: enabled ? facts.archived : undefined }),
  useActiveSessions: () => ({ data: facts.active }),
  workstreamSessionsQuery: (id: string) => ({ queryKey: ["workstream-sessions", id], queryFn: async () => [] }),
}));

const selectAgentTab = vi.hoisted(() => vi.fn(async () => {}));
vi.mock("@/features/search/selectAgentTab", () => ({ selectAgentTab }));

vi.mock("@/features/lead/lead", () => ({
  useProjectLead: () =>
    facts.lead
      ? { view: { projectPath: "/repo", workstreamId: "main", sessionId: "s", driving: [] }, settings: {} }
      : null,
  setLeadDockExpanded: vi.fn(),
  useLeadDockExpanded: () => false,
  offerLeadDraft: vi.fn(),
}));

const fresh = useTourStore.getInitialState();

function anchor(id: string) {
  const el = document.createElement("div");
  el.dataset.tour = id;
  el.innerHTML = "<button>inside</button>";
  el.getBoundingClientRect = () => ({ left: 400, top: 700, width: 600, height: 80, right: 1000, bottom: 780, x: 400, y: 700, toJSON() {} });
  document.body.appendChild(el);
  return el;
}

beforeEach(() => {
  useTourStore.setState(fresh, true);
  Object.assign(facts, { lead: true, connected: true, engine: true, projects: [{ path: "/repo" }], archived: [], active: [] });
  selectAgentTab.mockClear();
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => setTimeout(() => cb(0), 0));
  vi.stubGlobal("cancelAnimationFrame", (id: number) => clearTimeout(id));
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
});

afterEach(() => {
  cleanup();
  document.body.innerHTML = "";
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

function startAt(id: string) {
  const steps = pathSteps("firstRun");
  startTour("firstRun", steps);
  while (activeTour(useTourStore.getState())?.step.id !== id) nextStep();
}

describe("TourHost", () => {
  it("points at the anchor and moves focus to the coach mark's title", async () => {
    anchor("sidebar.project");
    startAt("fr.open");
    render(<TourHost />);
    const title = await screen.findByRole("heading", { name: "Open your project’s Lead" });
    expect(document.activeElement).toBe(title);
    const dialog = screen.getByRole("dialog");
    expect(dialog.getAttribute("aria-modal")).toBe("false");
    expect(document.querySelector("[data-tour-highlight]")).not.toBeNull();
  });

  it("shows the step unanchored, with a way back, when its anchor never mounts", async () => {
    vi.useFakeTimers();
    startAt("fr.open");
    render(<TourHost />);
    expect(screen.queryByRole("dialog")).toBeNull();
    await act(async () => {
      vi.advanceTimersByTime(ANCHOR_WAIT_MS + 50);
    });
    expect(screen.getByText("What this step points at isn’t on screen.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Take me there" })).toBeTruthy();
    expect(document.querySelector("[data-tour-highlight]")).toBeNull();
  });

  it("says why when a need is unmet, and offers the fix", async () => {
    facts.engine = false;
    startAt("fr.prompt");
    render(<TourHost />);
    expect(await screen.findByText(/none is installed on this server yet/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Open Settings → Binaries" })).toBeTruthy();
    // A TRY IT step can't be finished blocked, only skipped.
    expect((screen.getByRole("button", { name: "Next" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getAllByRole("button", { name: "Skip this step" }).length).toBeGreaterThan(0);
  });

  it("waits on a server that's offline", async () => {
    facts.connected = false;
    startAt("fr.open");
    render(<TourHost />);
    expect(await screen.findByText(/The server isn’t answering/)).toBeTruthy();
  });

  it("Esc pauses the tour and keeps its place", async () => {
    anchor("sidebar.project");
    startAt("fr.open");
    render(<TourHost />);
    await screen.findByRole("dialog");
    act(() => {
      runCommand(ESCAPE_COMMAND_ID);
    });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(pausedTour(useTourStore.getState())?.step.id).toBe("fr.open");
  });

  it("F6 moves focus to the anchor and back", async () => {
    const el = anchor("sidebar.project");
    startAt("fr.open");
    render(<TourHost />);
    const dialog = await screen.findByRole("dialog");
    fireEvent.keyDown(dialog, { key: "F6" });
    expect(document.activeElement).toBe(el.querySelector("button"));
    fireEvent.keyDown(window, { key: "F6" });
    expect(document.activeElement).toBe(screen.getByRole("heading", { name: "Open your project’s Lead" }));
  });

  it("keeps Next shut on a TRY IT step until its signal fires, but Skip this step moves on", async () => {
    anchor("lead.dock");
    startAt("fr.prompt");
    render(<TourHost />);
    expect((await screen.findByRole("button", { name: "Next" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Skip this step" }));
    expect(activeTour(useTourStore.getState())?.step.id).toBe("fr.expand");
  });

  it("shows a step whose anchor is optional as a plain card when the anchor isn't there", async () => {
    vi.useFakeTimers();
    startTour("whatsNew", pathSteps("whatsNew"), "wn.resume");
    render(<TourHost />);
    await act(async () => {
      vi.advanceTimersByTime(ANCHOR_WAIT_MS + 50);
    });
    expect(screen.getByRole("heading", { name: "Agents pick up where they left off" })).toBeTruthy();
    expect(screen.queryByText("What this step points at isn’t on screen.")).toBeNull();
    expect(screen.queryByRole("button", { name: "Take me there" })).toBeNull();
  });

  it("finishes the first run when the Lead's agent came and went before its steps", async () => {
    anchor("lead.panel.running");
    anchor("lead.panel.host");
    startAt("fr.handoff");
    setTaskSentAt(1_000_000);
    facts.archived = [{ item: { id: "child", projectPath: "/repo", createdAt: 1_000_500, parentWorkstreamId: "main" } }];
    render(<TourHost />);
    await screen.findByText("Agent spawned");
    expect(useTourStore.getState().tourWorkstreamId).toBe("child");
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expect(
      await screen.findByText("The agent has already finished, and the Lead archived its workstream, so its tab is closed."),
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Take me there" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    await screen.findByRole("heading", { name: "See what the server is carrying" });
    fireEvent.click(screen.getByRole("button", { name: "Next" }));
    expect((await screen.findByRole("button", { name: "Finish" }) as HTMLButtonElement).disabled).toBe(false);
  });
});

describe("TourHost v2 steps", () => {
  it("opens with an intro card: centred over a scrim, no highlight, no missing-anchor notice", async () => {
    vi.useFakeTimers();
    startTour("firstRun", pathSteps("firstRun"));
    render(<TourHost />);
    await act(async () => {
      vi.advanceTimersByTime(ANCHOR_WAIT_MS + 50);
    });
    expect(screen.getByRole("heading", { name: "Meet your Lead Engineer" })).toBeTruthy();
    expect(document.querySelector("[data-tour-scrim]")).not.toBeNull();
    expect(document.querySelector("[data-tour-highlight]")).toBeNull();
    expect(screen.queryByText("What this step points at isn’t on screen.")).toBeNull();
    expect(screen.queryByRole("button", { name: "Back" })).toBeNull();
  });

  it("lists the Lead's four jobs on the second intro card", async () => {
    startAt("fr.how");
    render(<TourHost />);
    await screen.findByRole("heading", { name: "What the Lead does for you" });
    expect(screen.getAllByRole("listitem")).toHaveLength(4);
    expect(screen.getByText("Lands it")).toBeTruthy();
  });

  it("points at this project's row in the sidebar, not another project's", async () => {
    facts.projects = [{ path: "/other" }, { path: "/repo" }];
    const other = anchor("sidebar.project");
    other.dataset.tourScope = "/other";
    const mine = anchor("sidebar.project");
    mine.dataset.tourScope = "/repo";
    mine.getBoundingClientRect = () => ({ left: 50, top: 100, width: 240, height: 36, right: 290, bottom: 136, x: 50, y: 100, toJSON() {} });
    startAt("fr.open");
    render(<TourHost />);
    await screen.findByRole("dialog");
    const hl = document.querySelector<HTMLElement>("[data-tour-highlight]")!;
    expect(hl.style.top).toBe("98px");
    fireEvent.click(mine.querySelector("button")!);
    expect(await screen.findByText("The Lead’s dock is open")).toBeTruthy();
  });

  it("outlines the dock's title line on the prompt step, labelled as the way to expand", async () => {
    anchor("lead.dock");
    anchor("lead.dock.header");
    startAt("fr.prompt");
    render(<TourHost />);
    await screen.findByRole("dialog");
    expect(document.querySelector("[data-tour-also]")).not.toBeNull();
    expect(screen.getByText("Click the title line or the arrow to expand")).toBeTruthy();
  });
});

describe("TourHost agent step", () => {
  it("opens the Lead's newest child session, in whichever workstream it runs", async () => {
    startAt("fr.agent");
    setTaskSentAt(1_000_000);
    facts.active = [{ session: { id: "child-s", workstreamId: "main", parentSessionId: "s", startedAt: 1_000_200 } }];
    render(<TourHost />);
    await act(async () => {});
    expect(selectAgentTab).toHaveBeenCalledWith("main", "child-s");
    expect(navigate).toHaveBeenCalledWith(expect.objectContaining({ search: { item: "main" } }));
  });
});

describe("host rules", () => {
  it("reads the project from the route, else the tour's, else the first", () => {
    const projects = [{ path: "/a" }, { path: "/b" }] as never;
    expect(routeProject("/projects/%2Fb")).toBe("/b");
    expect(tourProject("/b", "/a", projects)).toBe("/b");
    expect(tourProject(null, "/a", projects)).toBe("/a");
    expect(tourProject("/gone", null, projects)).toBe("/a");
    expect(tourProject(null, null, [])).toBeNull();
  });

  it("names the first unmet need in the step's order", () => {
    const all = { server: true, project: true, lead: true, engine: true, leadPanel: true };
    expect(firstUnmet(["server", "lead"], all)).toBeNull();
    expect(firstUnmet(["server", "lead"], { ...all, lead: false, server: false })?.need).toBe("server");
  });

  it("finds an engine only when one is installed and recent enough", () => {
    expect(hasEngine([{ kind: "codex", state: { state: "ok" } } as never])).toBe(true);
    expect(hasEngine([{ kind: "claude", state: { state: "tooOld", min: "1" } } as never])).toBe(false);
    expect(hasEngine([{ kind: "git", state: { state: "ok" } } as never])).toBe(false);
  });
});
