import { describe, expect, it } from "vitest";

import { planTaskTimeline, planTasksFromItems, todoPhaseLabel, type PlanTask } from "./planTasks";
import type { TranscriptItem } from "./transcript";


function makeTool(
  name: string,
  input: string,
  toolUseId: string,
  result: { content: string; isError: boolean } | null = null,
): Extract<TranscriptItem, { kind: "tool" }> {
  return { kind: "tool", id: `id-${toolUseId}`, toolUseId, name, input, result };
}

const ok = (content: string) => ({ content, isError: false });

function taskCreate(
  toolUseId: string,
  subject: string,
  opts: { description?: string; id?: number | null } = {},
): Extract<TranscriptItem, { kind: "tool" }> {
  const input: Record<string, unknown> = { subject };
  if (opts.description !== undefined) input.description = opts.description;
  const result =
    opts.id === null ? null : ok(`Task #${opts.id ?? 1} created successfully: ${subject}`);
  return makeTool("TaskCreate", JSON.stringify(input), toolUseId, result);
}

function taskUpdate(
  toolUseId: string,
  fields: { taskId: string; status?: string; subject?: string },
): Extract<TranscriptItem, { kind: "tool" }> {
  return makeTool("TaskUpdate", JSON.stringify(fields), toolUseId, ok(`Updated task #${fields.taskId} status`));
}

function taskDelete(toolUseId: string, taskId: string): Extract<TranscriptItem, { kind: "tool" }> {
  return makeTool(
    "TaskUpdate",
    JSON.stringify({ taskId, status: "deleted" }),
    toolUseId,
    ok(`Task #${taskId} deleted`),
  );
}

function taskList(toolUseId: string, snapshot: string): Extract<TranscriptItem, { kind: "tool" }> {
  return makeTool("TaskList", "{}", toolUseId, ok(snapshot));
}

function todoWrite(
  toolUseId: string,
  todos: Array<{ content: string; status: string; activeForm?: string }>,
): Extract<TranscriptItem, { kind: "tool" }> {
  return makeTool("TodoWrite", JSON.stringify({ todos }), toolUseId, ok("Todos updated"));
}


describe("planTasksFromItems", () => {
  it("returns empty for an empty transcript", () => {
    expect(planTasksFromItems([])).toEqual([]);
  });

  it("ignores non-tool and unrelated tool items", () => {
    const items: TranscriptItem[] = [
      { kind: "user", id: "u1", text: "hi", stage: null, attachments: [], createdAt: null, promptName: null },
      { kind: "assistant", id: "a1", text: "ok", streaming: false },
      makeTool("Bash", JSON.stringify({ command: "ls" }), "t1", ok("done")),
    ];
    expect(planTasksFromItems(items)).toEqual([]);
  });

  it("adds tasks from TaskCreate, taking the id from the result", () => {
    const items: TranscriptItem[] = [
      taskCreate("t1", "Build parser", { id: 1, description: "parse stream" }),
      taskCreate("t2", "Wire UI", { id: 2 }),
    ];
    expect(planTasksFromItems(items)).toEqual<PlanTask[]>([
      { id: "1", subject: "Build parser", status: "pending" },
      { id: "2", subject: "Wire UI", status: "pending" },
    ]);
  });

  it("shows a create optimistically (id null) before its result arrives", () => {
    const items: TranscriptItem[] = [taskCreate("t1", "Build parser", { id: null })];
    expect(planTasksFromItems(items)).toEqual<PlanTask[]>([
      { id: null, subject: "Build parser", status: "pending" },
    ]);
  });

  it("applies TaskUpdate status transitions by taskId, keeping order", () => {
    const items: TranscriptItem[] = [
      taskCreate("t1", "Build parser", { id: 1 }),
      taskCreate("t2", "Wire UI", { id: 2 }),
      taskUpdate("t3", { taskId: "1", status: "in_progress" }),
      taskUpdate("t4", { taskId: "1", status: "completed" }),
    ];
    expect(planTasksFromItems(items)).toEqual<PlanTask[]>([
      { id: "1", subject: "Build parser", status: "completed" },
      { id: "2", subject: "Wire UI", status: "pending" },
    ]);
  });

  it("lets TaskUpdate change a task's subject too", () => {
    const items: TranscriptItem[] = [
      taskCreate("t1", "Old subject", { id: 1 }),
      taskUpdate("t2", { taskId: "1", subject: "New subject" }),
    ];
    expect(planTasksFromItems(items)[0]).toMatchObject({ id: "1", subject: "New subject" });
  });

  it("resyncs from a TaskList snapshot", () => {
    const items: TranscriptItem[] = [
      taskList("t1", "#1 [in_progress] Build parser\n#2 [pending] Wire UI"),
    ];
    expect(planTasksFromItems(items)).toEqual<PlanTask[]>([
      { id: "1", subject: "Build parser", status: "in_progress" },
      { id: "2", subject: "Wire UI", status: "pending" },
    ]);
  });

  it("removes a task when TaskUpdate sets status to deleted", () => {
    const items: TranscriptItem[] = [
      taskCreate("t1", "Keep me", { id: 1 }),
      taskCreate("t2", "Delete me", { id: 2 }),
      taskDelete("t3", "2"),
    ];
    expect(planTasksFromItems(items)).toEqual<PlanTask[]>([
      { id: "1", subject: "Keep me", status: "pending" },
    ]);
  });

  it("drops every task when all are deleted, leaving an empty list", () => {
    const items: TranscriptItem[] = [
      taskCreate("t1", "One", { id: 1 }),
      taskCreate("t2", "Two", { id: 2 }),
      taskCreate("t3", "Three", { id: 3 }),
      taskDelete("t4", "1"),
      taskDelete("t5", "2"),
      taskDelete("t6", "3"),
    ];
    expect(planTasksFromItems(items)).toEqual([]);
  });

  it("does not count deleted tasks after a recreate (the 'TASKS 3/6' bug)", () => {
    const items: TranscriptItem[] = [
      taskCreate("t1", "Design the data model", { id: 1 }),
      taskCreate("t2", "Implement the API endpoints", { id: 2 }),
      taskCreate("t3", "Write integration tests", { id: 3 }),
      taskUpdate("t4", { taskId: "1", status: "completed" }),
      taskUpdate("t5", { taskId: "2", status: "in_progress" }),
      taskDelete("t6", "1"),
      taskDelete("t7", "2"),
      taskDelete("t8", "3"),
      taskList("t9", "No tasks found"),
      taskCreate("t10", "Design the data model", { id: 4 }),
      taskCreate("t11", "Implement the API endpoints", { id: 5 }),
      taskCreate("t12", "Write integration tests", { id: 6 }),
      taskUpdate("t13", { taskId: "4", status: "completed" }),
      taskUpdate("t14", { taskId: "5", status: "completed" }),
      taskUpdate("t15", { taskId: "6", status: "completed" }),
    ];
    expect(planTasksFromItems(items)).toEqual<PlanTask[]>([
      { id: "4", subject: "Design the data model", status: "completed" },
      { id: "5", subject: "Implement the API endpoints", status: "completed" },
      { id: "6", subject: "Write integration tests", status: "completed" },
    ]);
  });

  it("ignores a delete for an unknown task id", () => {
    const items: TranscriptItem[] = [
      taskCreate("t1", "Keep me", { id: 1 }),
      taskDelete("t2", "99"),
    ];
    expect(planTasksFromItems(items)).toEqual<PlanTask[]>([
      { id: "1", subject: "Keep me", status: "pending" },
    ]);
  });

  it("ignores a TaskUpdate with no taskId", () => {
    const items: TranscriptItem[] = [makeTool("TaskUpdate", JSON.stringify({ status: "completed" }), "t1", ok("x"))];
    expect(planTasksFromItems(items)).toEqual([]);
  });

  it("does not apply a TaskCreate result that errored", () => {
    const items: TranscriptItem[] = [
      makeTool("TaskCreate", JSON.stringify({ subject: "Nope" }), "t1", { content: "boom", isError: true }),
    ];
    expect(planTasksFromItems(items)).toEqual<PlanTask[]>([{ id: null, subject: "Nope", status: "pending" }]);
  });

  it("handles the legacy TodoWrite shape, replacing the list wholesale", () => {
    const items: TranscriptItem[] = [
      todoWrite("t1", [
        { content: "Step 1", status: "completed" },
        { content: "Step 2", status: "in_progress" },
      ]),
      todoWrite("t2", [
        { content: "Step 1", status: "completed" },
        { content: "Step 2", status: "completed" },
        { content: "Step 3", status: "pending" },
      ]),
    ];
    expect(planTasksFromItems(items)).toEqual<PlanTask[]>([
      { id: "1", subject: "Step 1", status: "completed" },
      { id: "2", subject: "Step 2", status: "completed" },
      { id: "3", subject: "Step 3", status: "pending" },
    ]);
  });

  it("starts a fresh plan when new tasks are created after the previous batch completed", () => {
    const items: TranscriptItem[] = [
      taskCreate("t1", "Build primes", { id: 1 }),
      taskCreate("t2", "Test primes", { id: 2 }),
      taskUpdate("t3", { taskId: "1", status: "completed" }),
      taskUpdate("t4", { taskId: "2", status: "completed" }),
      taskCreate("t5", "Build fizzbuzz", { id: 3 }),
      taskCreate("t6", "Test fizzbuzz", { id: 4 }),
    ];
    expect(planTasksFromItems(items)).toEqual<PlanTask[]>([
      { id: "3", subject: "Build fizzbuzz", status: "pending" },
      { id: "4", subject: "Test fizzbuzz", status: "pending" },
    ]);
  });

  it("extends the current plan when a task is added while others are still open", () => {
    const items: TranscriptItem[] = [
      taskCreate("t1", "First", { id: 1 }),
      taskUpdate("t2", { taskId: "1", status: "in_progress" }),
      taskCreate("t3", "Second", { id: 2 }),
    ];
    expect(planTasksFromItems(items)).toEqual<PlanTask[]>([
      { id: "1", subject: "First", status: "in_progress" },
      { id: "2", subject: "Second", status: "pending" },
    ]);
  });
});

describe("planTaskTimeline", () => {
  const read = (id: string) => makeTool("Read", "{}", id, ok("…"));

  it("matches planTasksFromItems and points each task at its first tool call", () => {
    const items: TranscriptItem[] = [
      todoWrite("w1", [
        { content: "Phase 1: Scaffold", status: "in_progress" },
        { content: "Phase 2: Build", status: "pending" },
      ]),
      read("r1"),
      read("r2"),
      todoWrite("w2", [
        { content: "Phase 1: Scaffold", status: "completed" },
        { content: "Phase 2: Build", status: "in_progress" },
      ]),
      read("r3"),
    ];
    const timeline = planTaskTimeline(items);
    expect(timeline.map(({ jumpToolUseId: _, ...t }) => t)).toEqual(planTasksFromItems(items));
    expect(timeline.map((t) => t.jumpToolUseId)).toEqual(["r1", "r3"]);
  });

  it("falls back to the tracking call before any work follows it", () => {
    const timeline = planTaskTimeline([todoWrite("w1", [{ content: "A", status: "in_progress" }])]);
    expect(timeline[0].jumpToolUseId).toBe("w1");
  });

  it("is null for a task that never started", () => {
    const timeline = planTaskTimeline([todoWrite("w1", [{ content: "A", status: "pending" }]), read("r1")]);
    expect(timeline[0].jumpToolUseId).toBeNull();
  });

  describe("since a plan was presented", () => {
    const present = makeTool("ExitPlanMode", "{}", "plan", ok("approved"));
    const subjects = (items: TranscriptItem[]) => planTaskTimeline(items, "plan").map((t) => t.subject);

    it("drops tasks that existed before the plan", () => {
      const items = [taskCreate("c1", "Research", { id: 1 }), taskUpdate("u1", { taskId: "1", status: "completed" }), present];
      expect(subjects(items)).toEqual([]);
    });

    it("keeps tasks created after the plan, with their jump targets", () => {
      const items = [
        taskCreate("c1", "Research", { id: 1 }),
        present,
        taskCreate("c2", "Build", { id: 2 }),
        taskUpdate("u2", { taskId: "2", status: "in_progress" }),
        read("r1"),
      ];
      const timeline = planTaskTimeline(items, "plan");
      expect(timeline.map((t) => t.subject)).toEqual(["Build"]);
      expect(timeline[0].jumpToolUseId).toBe("r1");
    });

    it("keeps an old task out when a later TaskList or TaskUpdate names it", () => {
      const items = [
        taskCreate("c1", "Research", { id: 1 }),
        present,
        taskCreate("c2", "Build", { id: 2 }),
        taskList("l1", "#1 [completed] Research\n#2 [pending] Build"),
        taskUpdate("u1", { taskId: "1", status: "completed" }),
      ];
      expect(subjects(items)).toEqual(["Build"]);
    });

    it("keeps a TodoWrite list written after the plan, though its positional ids repeat", () => {
      const items = [
        todoWrite("w1", [{ content: "Research", status: "completed" }]),
        present,
        todoWrite("w2", [{ content: "Build", status: "pending" }]),
      ];
      expect(subjects(items)).toEqual(["Build"]);
    });

    it("drops a TodoWrite list that was never rewritten after the plan", () => {
      expect(subjects([todoWrite("w1", [{ content: "Research", status: "completed" }]), present])).toEqual([]);
    });

    it("keeps a finished task when the next one is created after it", () => {
      const items = [
        present,
        taskCreate("c1", "Phase 1: Greet", { id: 1 }),
        taskUpdate("u1", { taskId: "1", status: "completed" }),
        taskCreate("c2", "Phase 2: Sign off", { id: 2 }),
      ];
      const timeline = planTaskTimeline(items, "plan");
      expect(timeline.map((t) => [t.subject, t.status])).toEqual([
        ["Phase 1: Greet", "completed"],
        ["Phase 2: Sign off", "pending"],
      ]);
      expect(planTaskTimeline(items.slice(1)).map((t) => t.subject)).toEqual(["Phase 1: Greet", "Phase 2: Sign off"]);
    });

    it("keeps everything when the plan's tool call is not in the transcript", () => {
      const items = [taskCreate("c1", "Research", { id: 1 })];
      expect(planTaskTimeline(items, "missing").map((t) => t.subject)).toEqual(["Research"]);
      expect(planTaskTimeline(items, null).map((t) => t.subject)).toEqual(["Research"]);
    });
  });
});

describe("a subagent's own todo list", () => {
  const child = <T extends TranscriptItem>(item: T): T => ({ ...item, parentId: "agent-1" });
  const items: TranscriptItem[] = [
    taskCreate("c1", "Phase 1: Scaffold", { id: 1 }),
    taskCreate("c2", "Phase 2: Build", { id: 2 }),
    taskUpdate("u1", { taskId: "1", status: "in_progress" }),
    makeTool("Agent", JSON.stringify({ subagent_type: "foreman", run_in_background: false }), "agent-1"),
    child(
      todoWrite("w1", [
        { content: "Write the route", status: "in_progress" },
        { content: "Add types", status: "pending" },
      ]),
    ),
    child(makeTool("Read", "{}", "r1", ok("…"))),
  ];

  it("leaves the session's tasks alone", () => {
    expect(planTasksFromItems(items)).toEqual<PlanTask[]>([
      { id: "1", subject: "Phase 1: Scaffold", status: "in_progress" },
      { id: "2", subject: "Phase 2: Build", status: "pending" },
    ]);
  });

  it("lets the session's next update land on its own task", () => {
    const next = [...items, taskUpdate("u2", { taskId: "1", status: "completed" })];
    expect(planTasksFromItems(next)[0]).toEqual({ id: "1", subject: "Phase 1: Scaffold", status: "completed" });
  });

  it("keeps the timeline on the session's tasks, jumping to the spawn", () => {
    expect(planTaskTimeline(items).map((t) => [t.subject, t.status, t.jumpToolUseId])).toEqual([
      ["Phase 1: Scaffold", "in_progress", "agent-1"],
      ["Phase 2: Build", "pending", null],
    ]);
  });
});

describe("todoPhaseLabel", () => {
  it("reads phase-level todos", () => {
    expect(todoPhaseLabel("Phase 2: Build the tracker")).toBe("2");
    expect(todoPhaseLabel("phase 4b — split")).toBe("4b");
    expect(todoPhaseLabel("[P3] wire it")).toBe("3");
  });

  it("ignores task tags and ordinary subjects", () => {
    expect(todoPhaseLabel("[P3.1] wire it")).toBeNull();
    expect(todoPhaseLabel("Rephase 2 things")).toBeNull();
    expect(todoPhaseLabel("Write tests")).toBeNull();
  });
});
