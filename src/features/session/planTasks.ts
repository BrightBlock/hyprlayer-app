
import type { TranscriptItem } from "./transcript";
import { parseToolInput } from "./toolDisplay";

export type PlanTaskStatus = "pending" | "in_progress" | "completed" | (string & {});

export type PlanTask = {
  id: string | null;
  subject: string;
  status: PlanTaskStatus;
};

function str(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return "";
}

function parseCreatedId(content: string): string | null {
  return /Task #(\d+)\b/.exec(content)?.[1] ?? null;
}

type IdentifiedTask = PlanTask & { id: string };

function parseTaskListSnapshot(content: string): IdentifiedTask[] {
  const out: IdentifiedTask[] = [];
  for (const line of content.split("\n")) {
    const m = /^#(\d+)\s+\[([^\]]+)\]\s+(.*)$/.exec(line.trim());
    if (m) out.push({ id: m[1], subject: m[3].trim(), status: m[2].trim() });
  }
  return out;
}

export function todosFromTodoWrite(input: string): IdentifiedTask[] {
  const parsed = parseToolInput(input);
  const todos = parsed?.todos;
  if (!Array.isArray(todos)) return [];
  return todos.map((raw, i) => {
    const t = (raw ?? {}) as Record<string, unknown>;
    return {
      id: String(i + 1),
      subject: str(t.content) || str(t.activeForm),
      status: str(t.status) || "pending",
    };
  });
}

type ToolItem = Extract<TranscriptItem, { kind: "tool" }>;

const TRACKING_TOOLS = new Set(["TaskCreate", "TaskUpdate", "TaskList", "TodoWrite"]);

// A subagent's TodoWrite is its own list; applied here it would replace the session's.
export function isSubagentTodoWrite(item: ToolItem): boolean {
  return item.name === "TodoWrite" && !!item.parentId;
}

// `newBatchClears`: a create after everything finished starts a fresh list, as the Tasks panel shows it.
export function applyTrackingTool(tasks: Map<string, PlanTask>, item: ToolItem, newBatchClears = true): void {
  switch (item.name) {
    case "TaskCreate": {
      if (newBatchClears && tasks.size > 0 && [...tasks.values()].every((t) => t.status === "completed")) {
        tasks.clear();
      }
      const subject = str(parseToolInput(item.input)?.subject);
      const id =
        item.result && !item.result.isError ? parseCreatedId(item.result.content) : null;
      const key = id ?? `pending:${item.toolUseId}`;
      const existing = tasks.get(key);
      tasks.set(key, {
        id,
        subject: subject || existing?.subject || "",
        status: existing?.status ?? "pending",
      });
      break;
    }

    case "TaskUpdate": {
      const parsed = parseToolInput(item.input);
      const id = str(parsed?.taskId);
      if (!id) break;
      const status = str(parsed?.status);
      if (status === "deleted") {
        tasks.delete(id);
        break;
      }
      const existing = tasks.get(id);
      tasks.set(id, {
        id,
        subject: str(parsed?.subject) || existing?.subject || "",
        status: status || existing?.status || "pending",
      });
      break;
    }

    case "TaskList": {
      if (item.result && !item.result.isError) {
        for (const t of parseTaskListSnapshot(item.result.content)) {
          const existing = tasks.get(t.id);
          tasks.set(t.id, { id: t.id, subject: t.subject || existing?.subject || "", status: t.status });
        }
      }
      break;
    }

    case "TodoWrite": {
      tasks.clear();
      for (const t of todosFromTodoWrite(item.input)) tasks.set(t.id, t);
      break;
    }
  }
}

export function planTasksFromItems(items: TranscriptItem[]): PlanTask[] {
  const tasks = new Map<string, PlanTask>();
  for (const item of items) {
    if (item.kind === "tool" && !isSubagentTodoWrite(item)) applyTrackingTool(tasks, item);
  }
  return Array.from(tasks.values());
}

export type TimelineTask = PlanTask & { jumpToolUseId: string | null };

// A task's jump target is the first top-level tool call after it went in_progress,
// else the tracking call itself while nothing has followed it yet.
// With `since`, tasks that existed at that tool call are dropped; they predate the plan it presented.
export function planTaskTimeline(items: TranscriptItem[], since: string | null = null): TimelineTask[] {
  const tasks = new Map<string, PlanTask>();
  const started = new Map<string, string>();
  const stale = new Set<string>();
  let awaitingWork: string[] = [];
  for (const item of items) {
    if (item.kind !== "tool" || isSubagentTodoWrite(item)) continue;
    if (item.toolUseId === since) for (const key of tasks.keys()) stale.add(key);
    if (!TRACKING_TOOLS.has(item.name)) {
      if (item.parentId) continue;
      for (const subject of awaitingWork) started.set(subject, item.toolUseId);
      awaitingWork = [];
      continue;
    }
    // TodoWrite replaces the list and reuses positional ids, so nothing it writes is stale.
    if (item.name === "TodoWrite") stale.clear();
    // A plan's finished phases stay finished when the next phase's task is created.
    applyTrackingTool(tasks, item, false);
    for (const t of tasks.values()) {
      if (t.status === "in_progress" && !started.has(t.subject)) {
        started.set(t.subject, item.toolUseId);
        awaitingWork.push(t.subject);
      }
    }
  }
  return Array.from(tasks)
    .filter(([key]) => !stale.has(key))
    .map(([, t]) => ({ ...t, jumpToolUseId: started.get(t.subject) ?? null }));
}

// `Phase 2: …` (implement_plan's one-todo-per-phase) or a `[P2]` tag; `[P2.3]` names a task instead.
export function todoPhaseLabel(subject: string): string | null {
  const tag = /^\s*\[P(\d+[a-z]?)(\.\d+)?\]/i.exec(subject);
  if (tag) return tag[2] ? null : tag[1];
  return /^\s*phase\s+(\d+[a-z]?)\b/i.exec(subject)?.[1] ?? null;
}
