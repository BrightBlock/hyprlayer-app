import type { PlanDocPhase } from "@/features/plan/planDoc";
import { matchTodoToPlan } from "@/features/plan/planTracker";
import { applyTrackingTool, isSubagentTodoWrite, type PlanTask } from "@/features/session/planTasks";
import { parseToolInput, toolFilePath } from "@/features/session/toolDisplay";
import type { TranscriptItem } from "@/features/session/transcript";

export type TaskRef = { phaseLabel: string | null; taskKey: string | null; subject: string };
export type Attribution = { byFile: Map<string, TaskRef[]>; tracked: boolean };

type ToolItem = Extract<TranscriptItem, { kind: "tool" }>;
type Parsed = { kind: "track" } | { kind: "edit"; paths: string[] } | null;

const TRACKING_TOOLS = new Set(["TaskCreate", "TaskUpdate", "TaskList", "TodoWrite"]);
const EDIT_TOOLS = new Set(["Write", "Edit", "MultiEdit", "NotebookEdit"]);

// A result replaces its item, so a new object is parsed again and the old entry is collected.
const parseCache = new WeakMap<ToolItem, Parsed>();

function parseItem(item: ToolItem): Parsed {
  const cached = parseCache.get(item);
  if (cached !== undefined) return cached;
  let parsed: Parsed = null;
  if (TRACKING_TOOLS.has(item.name)) {
    parsed = isSubagentTodoWrite(item) ? null : { kind: "track" };
  } else if (EDIT_TOOLS.has(item.name) && item.result && !item.result.isError) {
    const paths = new Set<string>();
    const main = toolFilePath(item.name, item.input);
    if (main) paths.add(main);
    // A Codex patch carries one entry per file.
    const edits = item.name === "MultiEdit" ? parseToolInput(item.input)?.edits : null;
    if (Array.isArray(edits)) {
      for (const e of edits) {
        const p = (e as Record<string, unknown> | null)?.file_path;
        if (typeof p === "string" && p) paths.add(p);
      }
    }
    parsed = paths.size > 0 ? { kind: "edit", paths: [...paths] } : null;
  }
  parseCache.set(item, parsed);
  return parsed;
}

function relativeTo(root: string, path: string): string | null {
  const base = root.replace(/\\/g, "/").replace(/\/+$/, "");
  const p = path.replace(/\\/g, "/");
  let rel: string;
  if (p.startsWith(`${base}/`)) rel = p.slice(base.length + 1);
  else if (p.startsWith("/") || /^[A-Za-z]:\//.test(p)) return null;
  else rel = p.replace(/^(\.\/)+/, "");
  return rel && !rel.split("/").includes("..") ? rel : null;
}

function sameRef(a: TaskRef, b: TaskRef): boolean {
  return a.phaseLabel === b.phaseLabel && a.taskKey === b.taskKey && (a.phaseLabel !== null || a.subject === b.subject);
}

// Folds the whole transcript each time: a result lands on an earlier item, so an append-only fold would miss it.
export function attributeFiles(
  items: TranscriptItem[],
  phases: PlanDocPhase[],
  worktreePath: string,
  since: string | null,
): Attribution {
  const criteria = new Set(phases.flatMap((p) => p.criteria.map((c) => c.key)));
  const refs = new Map<string, TaskRef>();
  const refOf = (subject: string): TaskRef => {
    let ref = refs.get(subject);
    if (!ref) {
      const m = matchTodoToPlan(phases, subject);
      ref = m
        ? { phaseLabel: m.phaseLabel, taskKey: m.taskKey && !criteria.has(m.taskKey) ? m.taskKey : null, subject }
        : { phaseLabel: null, taskKey: null, subject };
      refs.set(subject, ref);
    }
    return ref;
  };

  const tasks = new Map<string, PlanTask>();
  const stale = new Set<string>();
  const started = new Map<string, number>();
  let active = new Set<string>();
  let clock = 0;
  let tracked = false;
  const byFile = new Map<string, TaskRef[]>();

  for (const item of items) {
    if (item.kind !== "tool") continue;
    const parsed = parseItem(item);
    if (item.toolUseId === since && !isSubagentTodoWrite(item)) for (const key of tasks.keys()) stale.add(key);
    if (parsed?.kind === "track") {
      // TodoWrite replaces the list and reuses positional ids, so nothing it writes is stale.
      if (item.name === "TodoWrite") stale.clear();
      applyTrackingTool(tasks, item, false);
      tracked = true;
      const next = new Set<string>();
      for (const t of tasks.values()) {
        if (t.status !== "in_progress") continue;
        next.add(t.subject);
        if (!active.has(t.subject)) started.set(t.subject, ++clock);
      }
      active = next;
      continue;
    }
    if (parsed?.kind !== "edit") continue;

    let owner: TaskRef | null = null;
    let ownerAt = -1;
    let fallback: TaskRef | null = null;
    let fallbackAt = -1;
    for (const [key, t] of tasks) {
      if (t.status !== "in_progress" || stale.has(key) || !t.subject) continue;
      const at = started.get(t.subject) ?? 0;
      const ref = refOf(t.subject);
      if (ref.phaseLabel !== null) {
        if (at > ownerAt) [owner, ownerAt] = [ref, at];
      } else if (at > fallbackAt) {
        [fallback, fallbackAt] = [ref, at];
      }
    }
    const ref = owner ?? fallback;
    if (!ref) continue;
    for (const abs of parsed.paths) {
      const rel = relativeTo(worktreePath, abs);
      if (!rel) continue;
      const list = byFile.get(rel);
      if (!list) byFile.set(rel, [ref]);
      else if (!list.some((r) => sameRef(r, ref))) list.push(ref);
    }
  }
  return { byFile, tracked };
}

export function tasksForFile(attribution: Attribution, file: { path: string; oldPath: string | null }): TaskRef[] {
  return attribution.byFile.get(file.path) ?? (file.oldPath ? attribution.byFile.get(file.oldPath) : undefined) ?? [];
}
