
import type { TranscriptItem } from "./transcript";
import { parseToolInput, subagentType } from "./toolDisplay";

export type BackgroundTaskStatus = "running" | "awaiting";

export type BackgroundTask = {
  taskId: string | null;
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

function parseTaskIdFromSpawnResult(content: string): string | null {
  const bash = /Command running in background with ID:\s*(\S+)\./.exec(content);
  if (bash) return bash[1];
  const workflow = /Task ID:\s*(\S+)/.exec(content);
  return workflow?.[1] ?? null;
}

export function isBackgroundSpawn(item: TranscriptItem): boolean {
  if (item.kind !== "tool") return false;
  if (item.name === "Workflow") return true;
  const agent = item.name === "Agent" || item.name === "Task";
  if (!agent && item.name !== "Bash") return false;
  const background = parseToolInput(item.input)?.run_in_background;
  // Agent backgrounds unless told not to; Bash is the other way round.
  return agent ? background !== false : background === true;
}

export function backgroundTasksFromItems(items: TranscriptItem[]): BackgroundTask[] {
  const hit = tasksCache.get(items);
  if (hit) return hit;
  const tasks = new Map<string, BackgroundTask>();
  const taskIdToToolUseId = new Map<string, string>();

  for (const item of items) {
    if (item.kind === "taskUpdate") {
      if (item.status === "running") continue;
      const toolUseId = item.toolUseId ?? taskIdToToolUseId.get(item.taskId);
      if (toolUseId) {
        tasks.delete(toolUseId);
        taskIdToToolUseId.delete(item.taskId);
      }
      continue;
    }

    if (item.kind === "relaunched" || item.kind === "exit") {
      tasks.clear();
      taskIdToToolUseId.clear();
      continue;
    }

    if (item.kind !== "tool") continue;

    if (item.parentId) continue;

    if (isBackgroundSpawn(item)) {
      if (item.result?.isError) continue;
      const parsed = parseToolInput(item.input);
      const descStr = str(parsed?.description);
      const cmdStr = str(parsed?.command);
      const type = subagentType(item.name, item.input);
      const label = type && descStr ? `${type}: ${descStr}` : type || descStr || firstLine(cmdStr);

      if (!tasks.has(item.toolUseId)) {
        tasks.set(item.toolUseId, {
          taskId: null,
          toolUseId: item.toolUseId,
          label,
          command: cmdStr,
          status: "running",
        });
      }

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

    if (item.name === "TaskStop" || item.name === "KillBash") {
      if (!item.result) continue;
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

    if (item.name === "TaskOutput" || item.name === "BashOutput") {
      const parsed = parseToolInput(item.input);
      const taskId = str(parsed?.task_id ?? parsed?.shell_id);
      if (!taskId) continue;
      const toolUseId = taskIdToToolUseId.get(taskId);
      if (!toolUseId) continue;
      const task = tasks.get(toolUseId);
      if (!task) continue;

      if (item.result === null) {
        task.status = "awaiting";
      } else if (item.result.content.includes("<status>completed</status>")) {
        tasks.delete(toolUseId);
        taskIdToToolUseId.delete(taskId);
      } else {
        task.status = "running";
      }
      continue;
    }

    for (const [taskId, toolUseId] of [...taskIdToToolUseId]) {
      if (item.input.includes(`${taskId}.output`)) {
        tasks.delete(toolUseId);
        taskIdToToolUseId.delete(taskId);
      }
    }
  }

  const out = Array.from(tasks.values());
  tasksCache.set(items, out);
  return out;
}

const tasksCache = new WeakMap<TranscriptItem[], BackgroundTask[]>();
