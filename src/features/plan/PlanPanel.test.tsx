// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { TimelineTask } from "@/features/session/planTasks";

import { PlanPanel } from "./PlanPanel";
import { parsePlanDoc } from "./planDoc";
import { trackPlan, type SessionSignal } from "./planTracker";

vi.mock("@/features/library/MarkdownPreview", () => ({
  MarkdownPreview: ({ body }: { body: string }) => <div>{body}</div>,
}));

const DOC = (status: string, checks: string) =>
  parsePlanDoc(
    `---\nstatus: ${status}\nauthor: jt\n---\n# OG images\n\n## Overview\n\nServe OG images.\n\n## Phase 1: Scaffold\n- [${checks[0]}] Add the route\n- [${checks[1]}] Add types\n\n## Phase 2: Build\n- [${checks[2]}] Port OG Images.html to the route\n- [${checks[3]}] Wire the image slot\n`,
    "/t/plans/og.md",
  );

const todo = (subject: string, status: string): TimelineTask => ({ id: null, subject, status, jumpToolUseId: "tu-1" });
const working: SessionSignal = { stage: "implement", activity: "working", reason: null, failureItemId: null };

function setReducedMotion(reduce: boolean) {
  window.matchMedia = vi.fn().mockImplementation((query: string) => ({
    matches: reduce && query.includes("reduce"),
    media: query,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  }));
}

function renderPanel(tracker: ReturnType<typeof trackPlan>, overrides: Partial<Parameters<typeof PlanPanel>[0]> = {}) {
  const props = {
    tracker,
    diff: { additions: 412, deletions: 18 },
    headerAction: { label: "Open in Library", icon: "external" as const, onClick: vi.fn() },
    onApprove: vi.fn(),
    onReviewChanges: vi.fn(),
    onJump: vi.fn(),
    ...overrides,
  };
  render(<PlanPanel {...props} />);
  return props;
}

beforeEach(() => setReducedMotion(false));
afterEach(cleanup);

describe("PlanPanel tracking", () => {
  const tracker = () =>
    trackPlan(DOC("active", "xx  "), [todo("Port OG Images.html to the route", "in_progress")], working);

  it("maps each task state to its box and text", () => {
    const failed = trackPlan(DOC("active", "x   "), [todo("Add types", "in_progress")], {
      ...working,
      activity: "failed",
      reason: "crashed",
      failureItemId: "i9",
    });
    renderPanel(failed);
    const done = screen.getByRole("button", { name: "Add the route, done" });
    expect(done.querySelector("[data-box]")?.className).toContain("bg-ok");
    expect(within(done).getByText("Add the route").className).toContain("line-through");

    const bad = screen.getByRole("button", { name: "Add types, failed: crashed" });
    expect(bad.querySelector("[data-box]")?.className).toContain("border-bad");
    expect(within(bad).getByText("crashed")).toBeTruthy();

    const pending = screen.getByRole("button", { name: "Wire the image slot, pending" });
    expect(pending.querySelector("[data-box]")?.className).toContain("border-edge");
    expect(within(pending).getByText("Wire the image slot").className).toContain("text-ink-4");
  });

  it("renders in progress and waiting rows", () => {
    renderPanel(tracker());
    const row = screen.getByRole("button", { name: "Port OG Images.html to the route, in progress" });
    expect(row.className).toContain("bg-panel-2");
    expect(row.querySelector("[data-box]")?.className).toContain("border-ink");
    expect(row.querySelector(".animate-plan-pulse")).not.toBeNull();
    cleanup();

    renderPanel(
      trackPlan(DOC("active", "    "), [todo("Wire the image slot", "in_progress")], {
        ...working,
        activity: "waiting",
        reason: "needs answer · image slot",
      }),
    );
    const waiting = screen.getByRole("button", { name: "Wire the image slot, waiting on you: needs answer · image slot" });
    expect(waiting.querySelector("[data-box]")?.className).toContain("border-warn");
  });

  it("labels phases with their state", () => {
    renderPanel(tracker());
    const done = screen.getByText("Phase 1 — Scaffold").closest("[data-phase-label]")!;
    expect(done.textContent).toContain("· done");
    expect(done.className).toContain("text-ok");
    expect(screen.getByText("Phase 2 — Build").closest("[data-phase-label]")!.textContent).toContain("· in progress");
  });

  it("shows the header, current task and footer", () => {
    renderPanel(tracker());
    expect(screen.getByText("OG images")).toBeTruthy();
    expect(screen.getByText("Build")).toBeTruthy();
    expect(screen.getByText("Phase 2 of 2")).toBeTruthy();
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe("63");
    expect(screen.getByText("Current task")).toBeTruthy();
    expect(screen.getByText("2 of 4 tasks done")).toBeTruthy();
    expect(screen.getByText("+412")).toBeTruthy();
    expect(screen.getByText("−18")).toBeTruthy();
  });

  it("hides the current task card when nothing is running", () => {
    renderPanel(trackPlan(DOC("active", "x   "), [], working));
    expect(screen.queryByText("Current task")).toBeNull();
  });

  it("jumps to a task's tool call on click", () => {
    const props = renderPanel(tracker());
    fireEvent.click(screen.getByRole("button", { name: "Port OG Images.html to the route, in progress" }));
    expect(props.onJump).toHaveBeenCalledWith({ toolUseId: "tu-1" });
  });

  it("switches to Done with Review changes when every task is finished", () => {
    const props = renderPanel(trackPlan(DOC("active", "xxxx"), [], working));
    expect(screen.getByText("Done").className).toContain("bg-ok");
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe("100");
    expect(screen.queryByText("Current task")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Review changes" }));
    expect(props.onReviewChanges).toHaveBeenCalled();
  });

  it("opens the plan in the Library", () => {
    const props = renderPanel(tracker());
    fireEvent.click(screen.getByRole("button", { name: "Open in Library" }));
    expect(props.headerAction!.onClick).toHaveBeenCalled();
  });

  it("drops the pulse and the bar transition under reduced motion", () => {
    setReducedMotion(true);
    renderPanel(tracker());
    expect(document.querySelector(".animate-plan-pulse")).toBeNull();
    const fill = screen.getByRole("progressbar").firstElementChild as HTMLElement;
    expect(fill.style.transition).toBe("");
  });
});

describe("PlanPanel draft", () => {
  it("shows the overview, chips and Approve & build", () => {
    const props = renderPanel(trackPlan(DOC("draft", "    "), [], { ...working, stage: "plan" }));
    expect(screen.getByText("Serve OG images.")).toBeTruthy();
    expect(screen.getByText("Plan")).toBeTruthy();
    expect(screen.getByText("Draft")).toBeTruthy();
    expect(screen.getByText("jt")).toBeTruthy();
    expect(screen.getByText("2 phases · 0 of 4 tasks done")).toBeTruthy();
    expect(screen.queryByRole("progressbar")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Approve & build" }));
    expect(props.onApprove).toHaveBeenCalled();
  });
});

describe("PlanPanel draft preview", () => {
  const draft = () => trackPlan(DOC("draft", "    "), [], { ...working, stage: "plan" });

  it("renders the spec's chips and non-interactive pending task rows", () => {
    renderPanel(draft());
    expect(screen.getByText("Draft").className).toContain("bg-tag-draft-bg");
    expect(screen.getByText("jt").className).toContain("text-ink-dim");
    const row = screen.getByText("Add the route").closest("[data-state]")!;
    expect(row.tagName).toBe("DIV");
    expect(row.getAttribute("data-state")).toBe("pending");
    expect(within(row as HTMLElement).getByText("Add the route").className).toContain("text-ink-2");
    expect(screen.queryByRole("button", { name: /Add the route/ })).toBeNull();
  });

  it("flashes only the changed sections", () => {
    renderPanel(draft(), { changed: new Set(["phase:2"]) });
    expect(screen.getByText("Phase 2 — Build").closest("section")!.className).toContain("animate-plan-flash");
    expect(screen.getByText("Phase 1 — Scaffold").closest("section")!.className).not.toContain("animate-plan-flash");
  });

  it("does not flash under reduced motion", () => {
    setReducedMotion(true);
    renderPanel(draft(), { changed: new Set(["phase:2", "overview"]) });
    expect(document.querySelector(".animate-plan-flash")).toBeNull();
  });

  it("disables Approve & build with a tooltip while the agent is writing", () => {
    const props = renderPanel(draft(), { approve: { blocked: "Still writing…" } });
    const approve = screen.getByRole("button", { name: "Approve & build" }) as HTMLButtonElement;
    expect(approve.disabled).toBe(true);
    expect(approve.parentElement!.getAttribute("title")).toBe("Still writing…");
    fireEvent.click(approve);
    expect(props.onApprove).not.toHaveBeenCalled();
  });

  it("closes on Escape from inside the panel", () => {
    const onClose = vi.fn();
    renderPanel(draft(), { onClose });
    fireEvent.keyDown(screen.getByRole("button", { name: "Open in Library" }), { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});

describe("PlanPanel for plans without phases", () => {
  const claude = (status: string | null) => {
    const doc = parsePlanDoc("# Plan creation flow\n\n## Context\n\nWhy we build it.\n\n### 1. Parser\n", "/Users/jt/.claude/plans/x.md");
    return trackPlan({ ...doc, status }, [], { ...working, stage: "plan" });
  };

  it("renders the body instead of an empty phase list", () => {
    renderPanel(claude(null));
    expect(screen.getByText(/Why we build it\./)).toBeTruthy();
    expect(screen.queryByText(/No phases found/)).toBeNull();
    expect(screen.queryByText(/phases ·/)).toBeNull();
  });

  it("shows a pending presentation as Awaiting approval, in the attention colour", () => {
    renderPanel(claude("awaiting approval"));
    expect(screen.getByText("Awaiting approval").className).toContain("bg-tag-draft-bg");
  });

  it("shows the source's own header action and hides Approve when there is nothing to approve", () => {
    const onClick = vi.fn();
    renderPanel(claude(null), { headerAction: { label: "Show in chat", icon: "chat", onClick }, approve: null });
    fireEvent.click(screen.getByRole("button", { name: "Show in chat" }));
    expect(onClick).toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: "Open in Library" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Approve & build" })).toBeNull();
  });

  it("switches an approved Claude plan to tracking, listing the agent's todos", () => {
    const doc = parsePlanDoc("# P\n\nBody\n", "/Users/jt/.claude/plans/x.md");
    const tracker = trackPlan({ ...doc, status: "approved" }, [todo("Write the parser", "in_progress")], working);
    renderPanel(tracker);
    expect(tracker.mode).toBe("tracking");
    expect(screen.getByRole("button", { name: "Write the parser, in progress, added by the agent" })).toBeTruthy();
  });
});
