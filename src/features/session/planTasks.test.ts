import { describe, expect, it } from "vitest";

import { planTasksFromItems, type PlanTask } from "./planTasks";
import type { TranscriptItem } from "./transcript";

// ─── helpers ────────────────────────────────────────────────────────────────

function makeTool(
  name: string,
  input: string,
  toolUseId: string,
  result: { content: string; isError: boolean } | null = null,
): Extract<TranscriptItem, { kind: "tool" }> {
  return { kind: "tool", id: `id-${toolUseId}`, toolUseId, name, input, result };
}

const ok = (content: string) => ({ content, isError: false });

/** A TaskCreate, with its result by default (id `#n`, echoing the subject). */
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

function taskList(toolUseId: string, snapshot: string): Extract<TranscriptItem, { kind: "tool" }> {
  return makeTool("TaskList", "{}", toolUseId, ok(snapshot));
}

function todoWrite(
  toolUseId: string,
  todos: Array<{ content: string; status: string; activeForm?: string }>,
): Extract<TranscriptItem, { kind: "tool" }> {
  return makeTool("TodoWrite", JSON.stringify({ todos }), toolUseId, ok("Todos updated"));
}

// ─── planTasksFromItems ───────────────────────────────────────────────────────

describe("planTasksFromItems", () => {
  it("returns empty for an empty transcript", () => {
    expect(planTasksFromItems([])).toEqual([]);
  });

  it("ignores non-tool and unrelated tool items", () => {
    const items: TranscriptItem[] = [
      { kind: "user", id: "u1", text: "hi", stage: null, attachments: [], createdAt: null },
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

  it("ignores a TaskUpdate with no taskId", () => {
    const items: TranscriptItem[] = [makeTool("TaskUpdate", JSON.stringify({ status: "completed" }), "t1", ok("x"))];
    expect(planTasksFromItems(items)).toEqual([]);
  });

  it("does not apply a TaskCreate result that errored", () => {
    const items: TranscriptItem[] = [
      makeTool("TaskCreate", JSON.stringify({ subject: "Nope" }), "t1", { content: "boom", isError: true }),
    ];
    // No id parsed → shown optimistically with a null id, never a phantom "#NaN".
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
      // A new plan in a later turn — the old completed batch should drop away.
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
      taskCreate("t3", "Second", { id: 2 }), // added mid-plan — First not done
    ];
    expect(planTasksFromItems(items)).toEqual<PlanTask[]>([
      { id: "1", subject: "First", status: "in_progress" },
      { id: "2", subject: "Second", status: "pending" },
    ]);
  });
});
