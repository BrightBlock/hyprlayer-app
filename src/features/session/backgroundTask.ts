/**
 * Pure detection of live background shell tasks from a session's transcript
 * items. No React, no IPC, no clock — fully deterministic and unit-testable,
 * exactly like transcript.ts / toolDisplay.ts.
 *
 * Background tasks are spawned by Bash{run_in_background:true} calls. Their
 * lifecycle: spawn → (optional TaskOutput polls) → natural completion
 * (TaskOutput returns <status>completed</status>) or TaskStop/KillBash.
 * Session exit is handled a layer up, not here: this pure walk still reports an
 * un-stopped spawn as live, so callers are responsible for dropping tasks once
 * the session is terminal (`useSessionBackgroundTasks` guards on `isTerminal`;
 * SessionChat hides the bar behind its terminal gate).
 *
 * Wire shapes (confirmed from real Claude Code 2.1.x sessions):
 *   Spawn result:
 *     "Command running in background with ID: {task_id}. Output is being
 *      written to: …"
 *   TaskOutput result — timeout/still-running:
 *     "<retrieval_status>timeout</retrieval_status>…<status>running</status>"
 *   TaskOutput result — success/completed:
 *     "<retrieval_status>success</retrieval_status>…<status>completed</status>
 *      …<output>…</output>"
 *   TaskStop result:
 *     JSON string: {"message":"Successfully stopped task: {id} ({cmd})",
 *                   "task_id":"{id}","task_type":"local_bash","command":"…"}
 */

import type { TranscriptItem } from "./transcript";
import { parseToolInput } from "./toolDisplay";

export type BackgroundTaskStatus = "running" | "awaiting";

export type BackgroundTask = {
  /** The Claude task/shell id (parsed from the spawn's result), or null until
   *  the spawn result arrives. */
  taskId: string | null;
  /** The spawning tool item's id — a stable key + the jump-to-transcript target. */
  toolUseId: string;
  /** Human label: the Bash `description`, else the first line of `command`. */
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

/** A Bash tool call with run_in_background:true — the spawn event. */
export function isBackgroundSpawn(item: TranscriptItem): boolean {
  if (item.kind !== "tool" || item.name !== "Bash") return false;
  return parseToolInput(item.input)?.run_in_background === true;
}

/**
 * Live background tasks for a transcript, in spawn order. Walks `tool` items:
 *  - Bash{run_in_background:true} opens a task (taskId filled from its result);
 *  - TaskStop / KillBash for a known id ends it;
 *  - a TaskOutput/BashOutput result with <status>completed</status> ends it;
 *  - an in-flight TaskOutput/BashOutput (result === null) flips its target to
 *    `awaiting` — the agent is blocked waiting on it;
 *  - a timed-out TaskOutput (<status>running</status> in content) resets to
 *    `running`.
 * Returns only live (non-ended) tasks. Pure; ended tasks are dropped so the
 * caller never has to filter.
 */
export function backgroundTasksFromItems(items: TranscriptItem[]): BackgroundTask[] {
  const tasks = new Map<string, BackgroundTask>(); // keyed by toolUseId
  const taskIdToToolUseId = new Map<string, string>(); // task_id → toolUseId

  for (const item of items) {
    if (item.kind !== "tool") continue;

    // ── 1. Background Bash spawn ─────────────────────────────────────────────
    if (isBackgroundSpawn(item)) {
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
        // Agent is currently blocked — result hasn't arrived yet.
        task.status = "awaiting";
      } else if (item.result.content.includes("<status>completed</status>")) {
        // Task completed naturally — remove from live set.
        tasks.delete(toolUseId);
        taskIdToToolUseId.delete(taskId);
      } else {
        // Timeout (<status>running</status>) or non-blocking poll — task is
        // still live; reset to running in case it was previously awaiting.
        task.status = "running";
      }
      continue;
    }
  }

  return Array.from(tasks.values());
}
