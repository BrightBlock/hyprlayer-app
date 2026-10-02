import { describe, expect, it } from "vitest";

import type { TimelineTask } from "@/features/session/planTasks";

import { parsePlanDoc } from "./planDoc";
import { trackPlan, type SessionSignal } from "./planTracker";

const PATH = "/t/plans/p.md";

function plan(checks: [boolean, boolean, boolean, boolean] = [false, false, false, false], status = "active") {
  const b = (c: boolean) => (c ? "x" : " ");
  return parsePlanDoc(
    `---\nstatus: ${status}\n---\n# OG\n\n## Phase 1: Scaffold\n- [${b(checks[0])}] Add the route\n- [${b(checks[1])}] Add \`types\`\n\n## Phase 2: Build\n- [${b(checks[2])}] Port OG Images.html to the route\n- [${b(checks[3])}] Wire the image slot\n`,
    PATH,
  );
}

function todo(subject: string, status: string, jumpToolUseId: string | null = null): TimelineTask {
  return { id: null, subject, status, jumpToolUseId };
}

const building: SessionSignal = {
  stage: "implement",
  activity: "working",
  reason: null,
  failureItemId: null,
  subagent: null,
};

describe("progress", () => {
  it("is zero, not NaN, for a plan with no tasks", () => {
    const t = trackPlan(parsePlanDoc("## Phase 1: A\n", PATH), [], building);
    expect(t.total).toBe(0);
    expect(t.progress).toBe(0);
    expect(t.allDone).toBe(false);
  });

  it("counts doc checkboxes", () => {
    expect(trackPlan(plan(), [], building).progress).toBe(0);
    expect(trackPlan(plan([true, true, false, false]), [], building).progress).toBe(0.5);
  });

  it("gives half credit to the in-progress task", () => {
    const t = trackPlan(plan([true, true, false, false]), [todo("Port OG Images.html to the route", "in_progress")], building);
    expect(t.done).toBe(2);
    expect(t.progress).toBeCloseTo(0.625);
  });

  it("gives the phase being built half credit until its rows are ticked", () => {
    const t = trackPlan(plan(), [todo("Phase 1: Scaffold", "in_progress")], building);
    expect([t.done, t.total, t.progress]).toEqual([0, 4, 0.25]);
  });

  it("drops that credit once the phase todo completes", () => {
    const t = trackPlan(plan([true, false, false, false]), [todo("Phase 1: Scaffold", "completed")], building);
    expect(t.progress).toBe(0.25);
  });

  it("reaches 100% and clears the current task when all are done", () => {
    const t = trackPlan(plan([true, true, true, true]), [], building);
    expect(t.progress).toBe(1);
    expect(t.allDone).toBe(true);
    expect(t.current).toBeNull();
  });
});

describe("matching", () => {
  it("maps a Phase N todo to the phase, not a task", () => {
    const t = trackPlan(plan([true, true, false, false]), [todo("Phase 2: Build the tracker", "in_progress", "tu-9")], building);
    expect(t.phases[1].state).toBe("in_progress");
    expect(t.phases[1].tasks.every((x) => x.state === "pending")).toBe(true);
    expect(t.current).toMatchObject({ title: "Phase 2: Build the tracker", state: "in_progress", taskKey: null });
    expect(t.currentPhase).toBe(2);
    expect(t.phases[1].tasks[0].jump).toEqual({ toolUseId: "tu-9" });
  });

  it("matches by [P2.1] id tag", () => {
    const t = trackPlan(plan(), [todo("[P2.1] porting the page", "in_progress")], building);
    expect(t.phases[1].tasks[0].state).toBe("in_progress");
    expect(t.current?.taskKey).toBe("P2.1");
    expect(t.current?.title).toBe("Port OG Images.html to the route");
  });

  it("matches by normalized exact title", () => {
    const t = trackPlan(plan(), [todo("  add `TYPES`. ", "completed")], building);
    expect(t.phases[0].tasks[1].state).toBe("done");
  });

  it("gives duplicate titles to the first unmatched task in doc order", () => {
    const doc = parsePlanDoc("## Phase 1: A\n- [ ] Same\n- [ ] Same\n", PATH);
    const t = trackPlan(doc, [todo("Same", "completed"), todo("Same", "in_progress")], building);
    expect(t.phases[0].tasks.map((x) => x.state)).toEqual(["done", "in_progress"]);
  });

  it("appends an unknown todo to the current phase as added", () => {
    const t = trackPlan(
      plan([true, true, false, false]),
      [todo("Phase 2: Build", "in_progress"), todo("Regenerate bindings", "completed")],
      building,
    );
    const added = t.phases[1].tasks[t.phases[1].tasks.length - 1];
    expect(added).toMatchObject({ title: "Regenerate bindings", added: true, state: "done" });
    expect(t.total).toBe(5);
    expect(t.done).toBe(3);
  });

  it("never lets a todo un-check a doc [x]", () => {
    const t = trackPlan(plan([true, false, false, false]), [todo("Add the route", "pending")], building);
    expect(t.phases[0].tasks[0].state).toBe("done");
  });

  it("shows a completed todo on an unchecked task as done and asks for a write-back", () => {
    const t = trackPlan(plan(), [todo("Add the route", "completed")], building);
    expect(t.phases[0].tasks[0].state).toBe("done");
    expect(t.writeBacks).toEqual([{ key: "P1.1", title: "Add the route" }]);
    expect(trackPlan(plan([true, false, false, false]), [todo("Add the route", "completed")], building).writeBacks).toEqual([]);
  });

  it("returns a task to its doc status once its todo is dropped", () => {
    expect(trackPlan(plan(), [], building).phases[0].tasks[0].state).toBe("pending");
  });
});

describe("checks", () => {
  const doc = parsePlanDoc(
    "## Phase 1: A\n- [ ] Write it\n### Success Criteria:\n#### Automated Verification:\n- [x] fmt\n- [ ] clippy\n#### Manual Verification:\n- [ ] Looks right\n",
    PATH,
  );

  it("flags check rows with their group and counts them apart from tasks", () => {
    const t = trackPlan(doc, [todo("clippy", "completed")], building);
    expect(t.phases[0].tasks.map((x) => [x.title, x.check, x.group, x.state])).toEqual([
      ["Write it", false, null, "pending"],
      ["fmt", true, "automated", "done"],
      ["clippy", true, "automated", "done"],
      ["Looks right", true, "manual", "pending"],
    ]);
    expect([t.done, t.total]).toEqual([2, 4]);
    expect(t.checks).toEqual({ done: 2, total: 3 });
  });
});

describe("doc edited mid-build", () => {
  it("keeps a title match when a task is inserted before it", () => {
    const edited = parsePlanDoc(
      "## Phase 1: Scaffold\n- [ ] Brand new\n- [ ] Add the route\n- [ ] Add `types`\n",
      PATH,
    );
    const t = trackPlan(edited, [todo("Add the route", "in_progress")], building);
    expect(t.phases[0].tasks.map((x) => [x.title, x.state])).toEqual([
      ["Brand new", "pending"],
      ["Add the route", "in_progress"],
      ["Add types", "pending"],
    ]);
  });

  it("turns the todo of a deleted task into an added one", () => {
    const edited = parsePlanDoc("## Phase 1: Scaffold\n- [ ] Add `types`\n", PATH);
    const t = trackPlan(edited, [todo("Add the route", "in_progress")], building);
    expect(t.phases[0].tasks[t.phases[0].tasks.length - 1]).toMatchObject({ title: "Add the route", added: true, state: "in_progress" });
  });
});

describe("session status overlay", () => {
  it("marks the in-progress task waiting, with the reason", () => {
    const t = trackPlan(plan(), [todo("Wire the image slot", "in_progress")], {
      ...building,
      activity: "waiting",
      reason: "needs answer · image slot",
    });
    expect(t.phases[1].tasks[1]).toMatchObject({ state: "waiting", reason: "needs answer · image slot" });
    expect(t.current?.state).toBe("waiting");
    expect(t.pulsing).toBe(false);
  });

  it("marks it failed and links to the failure item", () => {
    const t = trackPlan(plan(), [todo("Wire the image slot", "in_progress")], {
      ...building,
      activity: "failed",
      reason: "crashed",
      failureItemId: "i42",
    });
    expect(t.phases[1].tasks[1]).toMatchObject({ state: "failed", reason: "crashed", jump: { itemId: "i42" } });
  });

  it("carries the running subagent on the current task", () => {
    const subagent = { toolUseId: "f1", agent: "foreman", background: false, tools: 3, lastTool: "Edit diff.rs", steps: null };
    const t = trackPlan(plan(), [todo("Phase 1: Scaffold", "in_progress")], { ...building, subagent });
    expect(t.current?.subagent).toEqual(subagent);
    expect(t.pulsing).toBe(true);
  });

  it("leaves in-progress static once the session stops working", () => {
    const t = trackPlan(plan(), [todo("Wire the image slot", "in_progress")], { ...building, activity: "idle" });
    expect(t.phases[1].tasks[1].state).toBe("in_progress");
    expect(t.pulsing).toBe(false);
    expect(trackPlan(plan(), [todo("Wire the image slot", "in_progress")], building).pulsing).toBe(true);
  });
});

describe("phases", () => {
  it("labels done, in progress and pending", () => {
    const t = trackPlan(plan([true, true, true, false]), [], building);
    expect(t.phases.map((p) => p.state)).toEqual(["done", "in_progress"]);
    expect(trackPlan(plan(), [], building).phases.map((p) => p.state)).toEqual(["pending", "pending"]);
  });

  it("handles a phase with no tasks", () => {
    const doc = parsePlanDoc("## Phase 1: Empty\n\n## Phase 2: B\n- [ ] b\n", PATH);
    const t = trackPlan(doc, [todo("Phase 1: Empty", "completed")], building);
    expect(t.phases[0].state).toBe("done");
    expect(t.currentPhase).toBe(2);
  });

  it("points at the first unfinished phase, or the last when all are done", () => {
    expect(trackPlan(plan([true, true, false, false]), [], building).currentPhase).toBe(2);
    expect(trackPlan(plan([true, true, true, true]), [], building).currentPhase).toBe(2);
  });
});

describe("mode", () => {
  it("drafts a draft plan outside an implement session", () => {
    expect(trackPlan(plan(undefined, "draft"), [], { ...building, stage: "plan" }).mode).toBe("draft");
    expect(trackPlan(plan(undefined, "draft"), [], null).mode).toBe("draft");
  });

  it("tracks an implement session, or an approved plan anywhere", () => {
    expect(trackPlan(plan(undefined, "draft"), [], building).mode).toBe("tracking");
    expect(trackPlan(plan(undefined, "active"), [], { ...building, stage: "plan" }).mode).toBe("tracking");
  });

  it("drafts a presented plan awaiting approval or sent back, even in an implement session", () => {
    expect(trackPlan(plan(undefined, "awaiting approval"), [], building).mode).toBe("draft");
    expect(trackPlan(plan(undefined, "changes requested"), [], building).mode).toBe("draft");
    expect(trackPlan(plan(undefined, "approved"), [], building).mode).toBe("tracking");
  });
});

describe("a plan whose phases have no tasks", () => {
  const bare = parsePlanDoc(
    `---\nstatus: approved\n---\n# Greeting\n\n## Phase 1: Greet\n- Say hello\n\n## Phase 2: Sign off\n- Say goodbye\n`,
    PATH,
  );

  it("starts at zero", () => {
    const t = trackPlan(bare, [], building);
    expect([t.progress, t.allDone, t.currentPhase]).toEqual([0, false, 1]);
  });

  it("measures progress in phases", () => {
    const t = trackPlan(bare, [todo("Phase 1: Greet", "completed"), todo("Phase 2: Sign off", "in_progress")], building);
    expect([t.progress, t.allDone, t.currentPhase]).toEqual([0.75, false, 2]);
  });

  it("is done once every phase is", () => {
    const t = trackPlan(bare, [todo("Phase 1: Greet", "completed"), todo("Phase 2: Sign off", "completed")], building);
    expect([t.progress, t.allDone, t.current]).toEqual([1, true, null]);
  });

  it("is never done while it is a draft", () => {
    const draft = parsePlanDoc(`---\nstatus: draft\n---\n# G\n\n## Phase 1: Greet\n- Say hello\n`, PATH);
    expect(trackPlan(draft, [todo("Phase 1: Greet", "completed")], null).allDone).toBe(false);
  });
});

describe("jump targets", () => {
  it("is null without todo history", () => {
    expect(trackPlan(plan(), [], building).phases[0].tasks[0].jump).toBeNull();
  });

  it("prefers the task's own todo over its phase's", () => {
    const t = trackPlan(
      plan(),
      [todo("Phase 1: Scaffold", "in_progress", "tu-phase"), todo("Add the route", "completed", "tu-task")],
      building,
    );
    expect(t.phases[0].tasks[0].jump).toEqual({ toolUseId: "tu-task" });
    expect(t.phases[0].tasks[1].jump).toEqual({ toolUseId: "tu-phase" });
  });
});
