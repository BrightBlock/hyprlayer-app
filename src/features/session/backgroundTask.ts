/**
 * Pure detection of live background shell tasks from a session's transcript.
 * No React/IPC/clock, deterministic and unit-testable, like transcript.ts.
 *
 * A task is spawned by Bash{run_in_background:true} and ends when: the agent
 * reads its `<taskId>.output` file (the common terminus, the spawn result
 * steers it there, and reading suppresses the async completion signal); a
 * terminal `taskUpdate` notification arrives (the never-read fire-and-forget
 * case); a TaskOutput poll returns <status>completed</status>; or TaskStop.
 * Session exit is handled by the caller, this walk still reports an un-ended
 * spawn as live, so callers drop tasks once the session is terminal.
 */

import type { TranscriptItem } from "./transcript";
import { parseToolInput } from "./toolDisplay";

export type BackgroundTaskStatus = "running" | "awaiting";

export type BackgroundTask = {
  /** Claude task/shell id, or null until the spawn result arrives. */
  taskId: string | null;
  /** Spawning tool item's id, stable key + jump-to-transcript target. */
  toolUseId: string;
  label: string;
  command: string;
  status: BackgroundTaskStatus;
};

function str(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return "";
}

function firstLine(value: string): string {
  const trimmed = value.trim();
  const nl = trimmed.indexOf("\n");
  return nl === -1 ? trimmed : `${trimmed.slice(0, nl).trim()} …`;
}

/**
 * Parse the task id from a background Bash spawn's result content.
 * Wire format (confirmed): "Command running in background with ID: {task_id}."
 */
function parseTaskIdFromSpawnResult(content: string): string | null {
  const match = /Command running in background with ID:\s*(\S+)\./.exec(content);
  return match?.[1] ?? null;
}

/** A Bash tool call with run_in_background:true, the spawn event. */
export function isBackgroundSpawn(item: TranscriptItem): boolean {
  if (item.kind !== "tool" || item.name !== "Bash") return false;
  return parseToolInput(item.input)?.run_in_background === true;
}

/**
 * Live (non-ended) background tasks for a transcript, in spawn order. Pure, the
 * per-branch rules for opening, ending, and `awaiting` a task are inline below.
 */
export function backgroundTasksFromItems(items: TranscriptItem[]): BackgroundTask[] {
  const tasks = new Map<string, BackgroundTask>(); // keyed by toolUseId
  const taskIdToToolUseId = new Map<string, string>(); // task_id → toolUseId

  for (const item of items) {
    // ── 0. taskUpdate: claude's async completion notification ────────────────
    // Ends a task the agent never polled (the fire-and-forget case the paths
    // below miss). Match by carried `toolUseId`, else the spawn-result task_id.
    if (item.kind === "taskUpdate") {
      if (item.status === "running") continue; // still live, nothing to drop
      const toolUseId = item.toolUseId ?? taskIdToToolUseId.get(item.taskId);
      if (toolUseId) {
        tasks.delete(toolUseId);
        taskIdToToolUseId.delete(item.taskId);
      }
      continue;
    }

    // Owning process gone (`relaunched`/`exit`): its shells are dead and can't
    // complete-notify, so drop everything tracked so far; later spawns survive.
    // Not per-turn `init` (process alive) nor `rewound` (same process, shells live).
    if (item.kind === "relaunched" || item.kind === "exit") {
      tasks.clear();
      taskIdToToolUseId.clear();
      continue;
    }

    if (item.kind !== "tool") continue;

    // ── 1. Background Bash spawn ─────────────────────────────────────────────
    if (isBackgroundSpawn(item)) {
      // A spawn that errored (denied / hook-blocked / bad command) never started
      // a shell, no task_id, nothing to complete it, so don't track a phantom.
      if (item.result?.isError) continue;
      const parsed = parseToolInput(item.input);
      const descStr = str(parsed?.description);
      const cmdStr = str(parsed?.command);
      const label = descStr || firstLine(cmdStr);

      if (!tasks.has(item.toolUseId)) {
        tasks.set(item.toolUseId, {
          taskId: null,
          toolUseId: item.toolUseId,
          label,
          command: cmdStr,
          status: "running",
        });
      }

      // Extract taskId from result if already arrived (usually immediate).
      if (item.result && !item.result.isError) {
        const taskId = parseTaskIdFromSpawnResult(item.result.content);
        const task = tasks.get(item.toolUseId);
        if (task && taskId && !task.taskId) {
          task.taskId = taskId;
          taskIdToToolUseId.set(taskId, item.toolUseId);
        }
      }
      continue;
    }

    // ── 2. TaskStop / KillBash → confirmed stop ends the task ────────────────
    if (item.name === "TaskStop" || item.name === "KillBash") {
      if (!item.result) continue; // not yet confirmed
      const parsed = parseToolInput(item.input);
      const taskId = str(parsed?.task_id ?? parsed?.shell_id);
      if (!taskId) continue;
      const toolUseId = taskIdToToolUseId.get(taskId);
      if (toolUseId) {
        tasks.delete(toolUseId);
        taskIdToToolUseId.delete(taskId);
      }
      continue;
    }

    // ── 3. TaskOutput / BashOutput → awaiting (result=null) or status update ─
    if (item.name === "TaskOutput" || item.name === "BashOutput") {
      const parsed = parseToolInput(item.input);
      const taskId = str(parsed?.task_id ?? parsed?.shell_id);
      if (!taskId) continue;
      const toolUseId = taskIdToToolUseId.get(taskId);
      if (!toolUseId) continue;
      const task = tasks.get(toolUseId);
      if (!task) continue;

      if (item.result === null) {
        // Agent is currently blocked, result hasn't arrived yet.
        task.status = "awaiting";
      } else if (item.result.content.includes("<status>completed</status>")) {
        // Task completed naturally, remove from live set.
        tasks.delete(toolUseId);
        taskIdToToolUseId.delete(taskId);
      } else {
        // Timeout (<status>running</status>) or non-blocking poll, task is
        // still live; reset to running in case it was previously awaiting.
        task.status = "running";
      }
      continue;
    }

    // ── 4. Direct read of a task's `<taskId>.output` file → end tracking ─────
    // Any tool whose INPUT references the file (Read/cat/grep/tail): the agent
    // consumed the output directly, which suppresses the async notification, so
    // this read is the only end-signal we'll get, without it the task lingers
    // as a phantom forever. Ids are unique tokens, so the basename can't collide.
    // Snapshot the map, we delete while iterating.
    for (const [taskId, toolUseId] of [...taskIdToToolUseId]) {
      if (item.input.includes(`${taskId}.output`)) {
        tasks.delete(toolUseId);
        taskIdToToolUseId.delete(taskId);
      }
    }
  }

  return Array.from(tasks.values());
}
