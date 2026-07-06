/**
 * Pure derivation of the agent's live plan/to-do list from a session's
 * transcript items. Newer builds use the `TaskCreate`/`TaskUpdate`/`TaskList`
 * tool family; older builds (or `CLAUDE_CODE_ENABLE_TASKS=0`) use legacy
 * `TodoWrite`. Both arrive as ordinary `tool` items, so no backend change.
 *
 * Wire shapes (Claude Code 2.1.195 stream-json):
 *   TaskCreate  input:  {"subject":"Build parser","description":"parse stream"}
 *               result: "Task #1 created successfully: Build parser"
 *   TaskUpdate  input:  {"taskId":"1","status":"in_progress"}
 *               result: "Updated task #1 status"
 *   TaskList    result: "#1 [in_progress] Build parser\n#2 [pending] Wire UI"
 *   TodoWrite   input:  {"todos":[{"content":"Step 1","status":"pending"},…]}
 */

import type { TranscriptItem } from "./transcript";
import { parseToolInput } from "./toolDisplay";

/** Claude's three plan-task states; kept open to a raw string so an unexpected
 *  status from a future build passes through instead of being dropped. */
export type PlanTaskStatus = "pending" | "in_progress" | "completed" | (string & {});

export type PlanTask = {
  /** The Claude-assigned task id ("1", "2", …), or null for a `TaskCreate` whose
   *  result (which carries the id) hasn't arrived yet — shown optimistically. */
  id: string | null;
  /** The task's display text (`subject`, or a TodoWrite todo's `content`). */
  subject: string;
  status: PlanTaskStatus;
};

function str(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return "";
}

/** The `#N` id from a `TaskCreate` result ("Task #1 created successfully: …"). */
function parseCreatedId(content: string): string | null {
  return /Task #(\d+)\b/.exec(content)?.[1] ?? null;
}

/** A plan task whose id is known (snapshot / todo rows always carry one). */
type IdentifiedTask = PlanTask & { id: string };

/** Parse a `TaskList` snapshot: each line is "#<id> [<status>] <subject>". */
function parseTaskListSnapshot(content: string): IdentifiedTask[] {
  const out: IdentifiedTask[] = [];
  for (const line of content.split("\n")) {
    const m = /^#(\d+)\s+\[([^\]]+)\]\s+(.*)$/.exec(line.trim());
    if (m) out.push({ id: m[1], subject: m[3].trim(), status: m[2].trim() });
  }
  return out;
}

/** A TodoWrite todo carries its text in `content` (or `activeForm` while in
 *  progress on some builds); status is one of the three plan states. */
function todosFromTodoWrite(input: string): IdentifiedTask[] {
  const parsed = parseToolInput(input);
  const todos = parsed?.todos;
  if (!Array.isArray(todos)) return [];
  return todos.map((raw, i) => {
    const t = (raw ?? {}) as Record<string, unknown>;
    return {
      // TodoWrite todos have no stable wire id — synthesize a 1-based index so
      // the list keys are stable across the wholesale replacements.
      id: String(i + 1),
      subject: str(t.content) || str(t.activeForm),
      status: str(t.status) || "pending",
    };
  });
}

/**
 * The agent's current plan/to-do list, in task order, folded from the tool items
 * (see the per-case comments below). Pure; safe to call on every render over the
 * live or replayed transcript. The list is conversation-scoped — unlike
 * background shells, it is NOT cleared at a `relaunched`/`exit` boundary.
 */
export function planTasksFromItems(items: TranscriptItem[]): PlanTask[] {
  // Insertion order tracks task order: creates arrive in id order, updates patch
  // in place, and snapshot lines are already ordered — so no post-sort needed.
  const tasks = new Map<string, PlanTask>(); // keyed by id, or "pending:<toolUseId>"

  for (const item of items) {
    if (item.kind !== "tool") continue;

    switch (item.name) {
      case "TaskCreate": {
        // A create that lands after the whole previous batch finished starts a
        // NEW plan — drop the completed batch so the list shows only the current
        // plan, not every plan the conversation ever ran. A create while tasks
        // are still open just extends the current plan.
        if (tasks.size > 0 && [...tasks.values()].every((t) => t.status === "completed")) {
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
        // `status:"deleted"` permanently removes a task — drop it from the list
        // rather than retaining it with a phantom "deleted" status, which would
        // otherwise render as a pending row and inflate the n/total count.
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
        // A wholesale replacement — the latest TodoWrite is the entire list.
        tasks.clear();
        for (const t of todosFromTodoWrite(item.input)) tasks.set(t.id, t);
        break;
      }
    }
  }

  return Array.from(tasks.values());
}
