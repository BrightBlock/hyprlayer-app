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

  it("keeps completed tasks in the list", () => {
    const items: TranscriptItem[] = [
      taskCreate("t1", "Done thing", { id: 1 }),
      taskUpdate("t2", { taskId: "1", status: "completed" }),
    ];
    const tasks = planTasksFromItems(items);
    expect(tasks).toHaveLength(1);
    expect(tasks[0].status).toBe("completed");
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

  it("handles a full create → update → list sequence", () => {
    const items: TranscriptItem[] = [
      taskCreate("t1", "Build parser", { id: 1, description: "parse stream" }),
      taskCreate("t2", "Wire UI", { id: 2 }),
      taskUpdate("t3", { taskId: "1", status: "in_progress" }),
      taskList("t4", "#1 [in_progress] Build parser\n#2 [pending] Wire UI"),
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
