// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { TimelineTask } from "@/features/session/planTasks";

import { PlanPanel } from "./PlanPanel";
import { parsePlanDoc } from "./planDoc";
import { usePlanPreviewStore } from "./planPreviewStore";
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
const working: SessionSignal = { stage: "implement", activity: "working", reason: null, failureItemId: null, subagent: null };

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

beforeEach(() => {
  setReducedMotion(false);
  usePlanPreviewStore.setState({ phaseOverrides: {} });
});
afterEach(cleanup);

describe("PlanPanel phase collapse", () => {
  const THREE = (checks: string) =>
    parsePlanDoc(
      `---\nstatus: active\n---\n# Three\n\n## Phase 1: A\n- [${checks[0]}] first\n\n## Phase 2: B\n- [${checks[1]}] second\n\n## Phase 3: C\n- [${checks[2]}] third\n`,
      "/t/plans/three.md",
    );
  const header = (title: string) => screen.getByText(title).closest("button")!;
  const expanded = () => ["Phase 1 — A", "Phase 2 — B", "Phase 3 — C"].map((t) => header(t).getAttribute("aria-expanded"));

  const completed = () => screen.getByRole("button", { name: /^Completed/ });

  it("opens only the current phase, with finished ones folded into Completed below", () => {
    renderPanel(trackPlan(THREE("x  "), [], working));
    expect(completed().getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByText("Phase 1 — A")).toBeNull();
    expect(header("Phase 3 — C").compareDocumentPosition(completed()) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    fireEvent.click(completed());
    expect(expanded()).toEqual(["false", "true", "false"]);
    expect(screen.queryByText("first")).toBeNull();
    expect(screen.getByText("second")).toBeTruthy();
  });

  it("toggles a phase from its header", () => {
    renderPanel(trackPlan(THREE("x  "), [], working));
    fireEvent.click(header("Phase 3 — C"));
    expect(screen.getByText("third")).toBeTruthy();
    fireEvent.click(header("Phase 2 — B"));
    expect(screen.queryByText("second")).toBeNull();
  });

  it("keeps a choice across remounts while the phase stays current", () => {
    renderPanel(trackPlan(THREE("   "), [], working));
    fireEvent.click(header("Phase 1 — A"));
    cleanup();
    renderPanel(trackPlan(THREE("   "), [], working));
    expect(expanded()).toEqual(["false", "false", "false"]);
  });

  it("collapses a finished phase and opens the next, dropping earlier choices", () => {
    renderPanel(trackPlan(THREE("   "), [], working));
    fireEvent.click(header("Phase 3 — C"));
    cleanup();
    renderPanel(trackPlan(THREE("x  "), [], working));
    fireEvent.click(completed());
    expect(expanded()).toEqual(["false", "true", "false"]);
  });

  it("collapses every phase once the plan is done", () => {
    renderPanel(trackPlan(THREE("xxx"), [], working));
    expect(expanded()).toEqual(["false", "false", "false"]);
  });

  it("shows every phase of a draft, still collapsible", () => {
    renderPanel(trackPlan(DOC("draft", "    "), [], null));
    expect(screen.getByText("Add the route")).toBeTruthy();
    expect(screen.getByText("Wire the image slot")).toBeTruthy();
    fireEvent.click(header("Phase 2 — Build"));
    expect(screen.queryByText("Wire the image slot")).toBeNull();
  });
});

describe("PlanPanel current phase", () => {
  // Phase 0's spike is deferred while Phase 1 is done.
  const SPIKE = parsePlanDoc(
    "---\nstatus: active\n---\n# Spike\n\n## Phase 0: Spike\n- [x] lint\n- [ ] spike up\n\n## Phase 1: Config\n- [x] a\n\n## Phase 2: SPA\n- [ ] typecheck\n",
    "/t/plans/spike.md",
  );

  it("leads with the current phase, named by its label, over an earlier one left open", () => {
    renderPanel(trackPlan(SPIKE, [todo("Phase 2B: dev loop", "in_progress")], working));
    expect(screen.getByText("Phase 2 · 3 of 3")).toBeTruthy();
    const spa = screen.getByText("Phase 2 — SPA").closest("button")!;
    const spike = screen.getByText("Phase 0 — Spike").closest("button")!;
    expect(spa.compareDocumentPosition(spike) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect([spa.getAttribute("aria-expanded"), spike.getAttribute("aria-expanded")]).toEqual(["true", "false"]);
    expect(screen.getByRole("button", { name: "Phase 2B: dev loop, in progress, added by the agent" })).toBeTruthy();
  });

  it("keeps the phase header when the current todo is the phase itself", () => {
    renderPanel(trackPlan(DOC("active", "xx  "), [todo("Phase 2: Build", "in_progress")], working));
    expect(screen.getByText("Phase 2 — Build").closest("button")!.getAttribute("aria-expanded")).toBe("true");
  });
});

describe("PlanPanel checks", () => {
  const CHECKS = (status: string, tasks: string) =>
    parsePlanDoc(
      `---\nstatus: ${status}\n---\n# Review\n\n## Phase 1: Diff\n${tasks}### Success Criteria:\n#### Automated Verification:\n- [x] fmt\n- [ ] clippy\n#### Manual Verification:\n- [ ] Looks right\n`,
      "/t/plans/review.md",
    );
  const follows = (a: HTMLElement, b: HTMLElement) => Boolean(a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING);

  it("labels automated and manual checks under their phase", () => {
    renderPanel(trackPlan(CHECKS("active", ""), [], working));
    const phase = within(screen.getByText("Phase 1 — Diff").closest("section")!);
    const automated = phase.getByText("Automated checks");
    const manual = phase.getByText("Manual checks");
    expect(follows(automated, phase.getByText("clippy"))).toBe(true);
    expect(follows(phase.getByText("clippy"), manual)).toBe(true);
    expect(follows(manual, phase.getByText("Looks right"))).toBe(true);
  });

  it("counts a criteria-only phase by its checks alone", () => {
    renderPanel(trackPlan(CHECKS("active", ""), [], working));
    expect(screen.queryByText("Implementation")).toBeNull();
    expect(screen.getByText("1 of 2 automated · 0 of 1 manual")).toBeTruthy();
  });

  it("counts tasks and checks apart when a plan has both", () => {
    renderPanel(trackPlan(CHECKS("active", "- [ ] Write it\n"), [], working));
    expect(screen.getByText("0 of 1 task · 1 of 2 automated · 0 of 1 manual")).toBeTruthy();
  });

  it("counts checks outside a verification group as checks", () => {
    const doc = parsePlanDoc("---\nstatus: active\n---\n# U\n\n## Phase 1: A\n### Success Criteria:\n- [x] fmt\n- [ ] clippy\n", "/t/plans/u.md");
    renderPanel(trackPlan(doc, [], working));
    expect(screen.getByText("1 of 2 checks")).toBeTruthy();
  });

  it("rolls up open manual checks from every phase with their phase", () => {
    const doc = parsePlanDoc(
      "---\nstatus: active\n---\n# Roll\n\n## Phase 1: Diff\n- [x] Write it\n### Success Criteria:\n#### Manual Verification:\n- [ ] Looks right\n- [x] Reads well\n\n## Phase 2: Polish\n- [ ] Tune it\n### Success Criteria:\n#### Manual Verification:\n- [ ] Feels fast\n",
      "/t/plans/roll.md",
    );
    renderPanel(trackPlan(doc, [], working));
    const rollup = screen.getByRole("region", { name: "Open manual checks" });
    const rows = within(rollup).getAllByRole("listitem");
    expect(rows.map((r) => r.textContent)).toEqual(["Looks rightPhase 1", "Feels fastPhase 2"]);
    expect(within(rows[0]).getByText("Phase 1").getAttribute("title")).toBe("Diff");
    expect(within(rollup).queryByText("Reads well")).toBeNull();
  });

  it("checks off a rolled-up manual check it can write to", () => {
    const doc = parsePlanDoc(
      "---\nstatus: active\n---\n# Roll\n\n## Phase 1: Diff\n- [x] Write it\n### Success Criteria:\n#### Manual Verification:\n- [ ] Looks right\n",
      "/t/plans/roll.md",
    );
    const onCheckOff = vi.fn();
    renderPanel(trackPlan(doc, [], working), { onCheckOff });
    fireEvent.click(screen.getByRole("button", { name: "Mark done: Looks right" }));
    expect(onCheckOff).toHaveBeenCalledWith(expect.objectContaining({ title: "Looks right" }));
    cleanup();
    renderPanel(trackPlan(doc, [], working));
    expect(screen.queryByRole("button", { name: "Mark done: Looks right" })).toBeNull();
  });

  it("shows no roll-up once every manual check is ticked", () => {
    const doc = parsePlanDoc(
      "---\nstatus: active\n---\n# Roll\n\n## Phase 1: Diff\n- [ ] Write it\n### Success Criteria:\n#### Manual Verification:\n- [x] Looks right\n",
      "/t/plans/roll.md",
    );
    renderPanel(trackPlan(doc, [], working));
    expect(screen.queryByRole("region", { name: "Open manual checks" })).toBeNull();
    expect(screen.getByText("0 of 1 task · 1 of 1 manual")).toBeTruthy();
  });

  it("uses the same wording in a draft", () => {
    renderPanel(trackPlan(CHECKS("draft", ""), [], null));
    expect(screen.getByText("Automated checks")).toBeTruthy();
    expect(screen.getByText("1 phase · 1 of 2 automated · 0 of 1 manual")).toBeTruthy();
  });
});

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
    fireEvent.click(screen.getByText("Phase 2 — Build"));
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
    fireEvent.click(screen.getByRole("button", { name: /^Completed/ }));
    const done = screen.getByText("Phase 1 — Scaffold").closest("[data-phase-label]")!;
    expect(done.textContent).not.toContain("· done");
    expect(done.className).toContain("text-ok");
    expect(screen.getByText("Phase 2 — Build").closest("[data-phase-label]")!.textContent).toContain("· in progress");
  });

  it("shows the header, counts, current task and diff", () => {
    renderPanel(tracker());
    expect(screen.getByText("OG images")).toBeTruthy();
    expect(screen.getByText("Build")).toBeTruthy();
    expect(screen.getByText("Phase 2 of 2")).toBeTruthy();
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe("63");
    expect(screen.getByText("Current task")).toBeTruthy();
    expect(screen.getByText("2 of 4 tasks")).toBeTruthy();
    expect(screen.getByText("+412")).toBeTruthy();
    expect(screen.getByText("−18")).toBeTruthy();
  });

  it("shows the running subagent's steps under the current task", () => {
    const subagent = { toolUseId: "f1", agent: "foreman", background: false, tools: 160, lastTool: "Edit diff.rs", steps: { done: 1, total: 3, current: "Diff layer" }, phase: "1" };
    renderPanel(trackPlan(DOC("active", "    "), [todo("Phase 1: Scaffold", "in_progress")], { ...working, subagent }));
    expect(screen.getByText("foreman · 1 of 3 steps · Diff layer")).toBeTruthy();
  });

  it("falls back to the subagent's tool count and last call", () => {
    const subagent = { toolUseId: "f1", agent: "foreman", background: false, tools: 160, lastTool: "Edit diff.rs", steps: null, phase: "1" };
    renderPanel(trackPlan(DOC("active", "    "), [todo("Phase 1: Scaffold", "in_progress")], { ...working, subagent }));
    expect(screen.getByText("foreman · 160 tools · Edit diff.rs")).toBeTruthy();
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
    expect(screen.getByText("2 phases · 0 of 4 tasks")).toBeTruthy();
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

describe("PlanPanel goals", () => {
  const draft = () => trackPlan(DOC("draft", "    "), [], { ...working, stage: "plan" });

  it("no build-until-met without a plan file", () => {
    renderPanel(draft());
    expect(screen.queryByRole("radiogroup", { name: "Build mode" })).toBeNull();
    expect(screen.queryByRole("radio", { name: "Build until met" })).toBeNull();
    cleanup();

    renderPanel(draft(), { buildGoal: { render: () => <div>until-met panel</div> } });
    expect(screen.getByRole("button", { name: "Approve & build" })).toBeTruthy();
    fireEvent.click(screen.getByRole("radio", { name: "Build until met" }));
    expect(screen.getByText("until-met panel")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Approve & build" })).toBeNull();
  });

  it("the phase action is a sibling", () => {
    const phaseAction = (phase: { label: string }) => <button type="button">Run phase {phase.label}</button>;
    renderPanel(trackPlan(DOC("active", "xx  "), [], working), { phaseAction });
    fireEvent.click(screen.getByRole("button", { name: /^Completed/ }));
    expect(screen.getByText("Phase 1 — Scaffold")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Run phase 1" })).toBeNull();
    const action = screen.getByRole("button", { name: "Run phase 2" });
    expect(action.parentElement!.closest("button")).toBeNull();
    const header = screen.getByText("Phase 2 — Build").closest("button")!;
    expect(header.contains(action)).toBe(false);
    expect(header.parentElement!.contains(action)).toBe(true);
  });
});
