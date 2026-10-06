import { basename } from "@/lib/path";

import { backgroundTasksFromItems, isBackgroundSpawn } from "./backgroundTask";
import { todosFromTodoWrite } from "./planTasks";
import type { TranscriptItem } from "./transcript";
import { parseToolInput, toolFilePath, toolSummary } from "./toolDisplay";

export type SubagentSteps = { done: number; total: number; current: string | null };

export type RunningSubagent = {
  toolUseId: string;
  agent: string;
  background: boolean;
  tools: number;
  lastTool: string | null;
  steps: SubagentSteps | null;
  // The plan phase it was spawned to build, from its description or prompt.
  phase: string | null;
};

type Tool = Extract<TranscriptItem, { kind: "tool" }>;
type Children = { tools: number; last: Tool | null; todos: Tool | null };

function str(value: unknown): string {
  return typeof value === "string" ? value : "";
}

const PHASE = /\bphase\s+(\d+[a-z]?)\b/i;

function phaseOf(input: Record<string, unknown> | null): string | null {
  return PHASE.exec(str(input?.description))?.[1] ?? PHASE.exec(str(input?.prompt))?.[1] ?? null;
}

function describeTool(item: Tool): string {
  const file = toolFilePath(item.name, item.input);
  const detail = file ? basename(file) : toolSummary(item.name, item.input);
  return detail ? `${item.name} ${detail}` : item.name;
}

function stepsOf(todoWrite: Tool | null): SubagentSteps | null {
  const todos = todoWrite ? todosFromTodoWrite(todoWrite.input) : [];
  if (todos.length === 0) return null;
  return {
    done: todos.filter((t) => t.status === "completed").length,
    total: todos.length,
    current: todos.find((t) => t.status === "in_progress")?.subject ?? null,
  };
}

// Walks backwards so the latest spawn wins; nothing before an exit or relaunch is still running.
export function runningSubagent(items: TranscriptItem[]): RunningSubagent | null {
  const hit = cache.get(items);
  if (hit !== undefined) return hit;
  const open = new Set(backgroundTasksFromItems(items).map((t) => t.toolUseId));
  const children = new Map<string, Children>();
  let found: RunningSubagent | null = null;
  for (let i = items.length - 1; i >= 0; i--) {
    const item = items[i];
    if (item.kind === "exit" || item.kind === "relaunched") break;
    if (item.kind !== "tool") continue;
    if (item.parentId) {
      const c = children.get(item.parentId) ?? { tools: 0, last: null, todos: null };
      c.tools++;
      c.last ??= item;
      if (item.name === "TodoWrite") c.todos ??= item;
      children.set(item.parentId, c);
      continue;
    }
    if (item.name !== "Agent" && item.name !== "Task") continue;
    const background = isBackgroundSpawn(item);
    if (background ? !open.has(item.toolUseId) : item.result !== null) continue;
    const input = parseToolInput(item.input);
    const c = children.get(item.toolUseId);
    found = {
      toolUseId: item.toolUseId,
      agent: str(input?.subagent_type) || str(input?.description) || item.name,
      background,
      tools: c?.tools ?? 0,
      lastTool: c?.last ? describeTool(c.last) : null,
      steps: stepsOf(c?.todos ?? null),
      phase: phaseOf(input),
    };
    break;
  }
  cache.set(items, found);
  return found;
}

const cache = new WeakMap<TranscriptItem[], RunningSubagent | null>();
