import { todoPhaseLabel, type TimelineTask } from "@/features/session/planTasks";
import type { RunningSubagent } from "@/features/session/subagent";
import type { TranscriptJump } from "@/features/session/transcriptJump";

import {
  phaseRows,
  type CheckGroup,
  type PlanDoc,
  type PlanDocCriterion,
  type PlanDocPhase,
  type PlanDocTask,
} from "./planDoc";

export type JumpTarget = TranscriptJump;
export type TaskState = "done" | "in_progress" | "pending" | "waiting" | "failed";
export type PhaseState = "done" | "in_progress" | "pending";

export type TrackedTask = {
  key: string;
  title: string;
  state: TaskState;
  added: boolean;
  check: boolean;
  group: CheckGroup | null;
  reason: string | null;
  jump: JumpTarget | null;
};

export type TrackedPhase = { label: string; name: string; state: PhaseState; tasks: TrackedTask[] };

export type CurrentItem = {
  title: string;
  state: "in_progress" | "waiting" | "failed";
  reason: string | null;
  taskKey: string | null;
  // The todo names the phase itself, so the card already says what its header would.
  phaseLevel: boolean;
  subagent: RunningSubagent | null;
};

export type SessionSignal = {
  stage: string;
  activity: "working" | "waiting" | "failed" | "idle";
  reason: string | null;
  failureItemId: string | null;
  subagent: RunningSubagent | null;
};

export type PlanTracker = {
  mode: "draft" | "tracking";
  title: string;
  status: string | null;
  author: string | null;
  overview: string;
  body: string;
  phases: TrackedPhase[];
  current: CurrentItem | null;
  currentPhase: number | null;
  done: number;
  total: number;
  checks: { done: number; total: number };
  progress: number;
  allDone: boolean;
  // Every phase built; unlike `allDone`, criteria and added rows don't count.
  tasksDone: boolean;
  pulsing: boolean;
  writeBacks: { key: string; title: string }[];
};

const TASK_TAG = /^\s*\[P(\d+[a-z]?)\.(\d+)\]\s*/i;
const ID_TAG = /^\s*\[P\d+[a-z]?(?:\.\d+)?\]\s*/i;

function normalize(text: string): string {
  return text
    .replace(ID_TAG, "")
    .replace(/`|\*\*/g, "")
    .toLowerCase()
    .replace(/\s+/g, " ")
    .trim()
    .replace(/[.:;,!]+$/, "")
    .trim();
}

type Slot = {
  key: string;
  title: string;
  checked: boolean;
  todo: TimelineTask | null;
  check: boolean;
  group: CheckGroup | null;
};
type Phase = { label: string; name: string; slots: Slot[]; todo: TimelineTask | null; added: TimelineTask[] };

export type PlanMatch = { phaseLabel: string; taskKey: string | null };

// `taken` lets the tracker skip rows another todo already holds; `taskKey: null` is a phase-level match.
export function matchTodoToPlan(
  phases: PlanDocPhase[],
  subject: string,
  taken: (key: string) => boolean = () => false,
): PlanMatch | null {
  const phaseByLabel = (label: string) => phases.find((p) => p.label.toLowerCase() === label.toLowerCase());
  const tag = TASK_TAG.exec(subject);
  const tagPhase = tag ? phaseByLabel(tag[1]) : undefined;
  const tagged =
    tag && tagPhase && phaseRows(tagPhase).find((r) => r.key.toLowerCase() === `p${tag[1]}.${tag[2]}`.toLowerCase());
  if (tagPhase && tagged && !taken(tagged.key)) return { phaseLabel: tagPhase.label, taskKey: tagged.key };
  const label = todoPhaseLabel(subject);
  const phase = label !== null ? phaseByLabel(label) : undefined;
  if (phase) return { phaseLabel: phase.label, taskKey: null };
  const wanted = normalize(subject);
  for (const p of phases) {
    const row = phaseRows(p).find((r) => !taken(r.key) && normalize(r.title) === wanted);
    if (row) return { phaseLabel: p.label, taskKey: row.key };
  }
  return null;
}

function slotOf(row: PlanDocTask | PlanDocCriterion): Slot {
  const check = "group" in row;
  return { key: row.key, title: row.title, checked: row.checked, todo: null, check, group: check ? row.group : null };
}

function taskStateOf(slot: Slot): TaskState {
  if (slot.checked || slot.todo?.status === "completed") return "done";
  return slot.todo?.status === "in_progress" ? "in_progress" : "pending";
}

function todoState(todo: TimelineTask): TaskState {
  if (todo.status === "completed") return "done";
  return todo.status === "in_progress" ? "in_progress" : "pending";
}

function phaseStateOf(phase: Phase, states: TaskState[]): PhaseState {
  const status = phase.todo?.status;
  if (states.length === 0) {
    if (status === "completed") return "done";
    return status === "in_progress" ? "in_progress" : "pending";
  }
  if (states.every((s) => s === "done")) return "done";
  if (status === "in_progress" || status === "completed" || states.some((s) => s !== "pending")) {
    return "in_progress";
  }
  return "pending";
}

function jumpOf(todo: TimelineTask | null): JumpTarget | null {
  return todo?.jumpToolUseId ? { toolUseId: todo.jumpToolUseId } : null;
}

// `approved` comes from an allowed ExitPlanMode or an approved inline plan, which build in the same session.
const TRACKING_STATUSES = new Set(["active", "implemented", "approved"]);
// A presented plan not yet approved outranks the implement stage: nothing has been built from it.
const UNAPPROVED_STATUSES = new Set(["awaiting approval", "changes requested"]);

// Structure comes only from the doc; todos contribute status, and unmatched ones are appended as `added`.
export function trackPlan(
  doc: PlanDoc,
  sessionTodos: TimelineTask[],
  session: SessionSignal | null,
): PlanTracker {
  const status = doc.status?.toLowerCase() ?? null;
  const mode =
    status !== null && UNAPPROVED_STATUSES.has(status)
      ? "draft"
      : session?.stage === "implement" || (status !== null && TRACKING_STATUSES.has(status))
        ? "tracking"
        : "draft";
  // A planning agent's own todos are research steps, not the plan's tasks.
  const todos = mode === "draft" ? [] : sessionTodos;
  const phases: Phase[] = doc.phases.map((p) => ({
    label: p.label,
    name: p.name,
    slots: phaseRows(p).map(slotOf),
    todo: null,
    added: [],
  }));
  const slots = phases.flatMap((p) => p.slots);
  const slotByKey = new Map(slots.map((s) => [s.key, s]));
  const unmatched: TimelineTask[] = [];

  const taken = (key: string) => slotByKey.get(key)?.todo != null;
  for (const todo of todos) {
    const match = matchTodoToPlan(doc.phases, todo.subject, taken);
    const slot = match?.taskKey != null ? slotByKey.get(match.taskKey) : undefined;
    const phase = match && match.taskKey === null ? phases.find((p) => p.label === match.phaseLabel) : undefined;
    if (slot) slot.todo = todo;
    else if (phase) phase.todo = todo;
    else unmatched.push(todo);
  }

  const inProgress = (p: Phase) =>
    p.todo?.status === "in_progress" || p.slots.some((s) => taskStateOf(s) === "in_progress");
  const finished = (p: Phase) => phaseStateOf(p, p.slots.map(taskStateOf)) === "done";
  let home = phases.findIndex(inProgress);
  if (home < 0) home = phases.findIndex((p) => !finished(p));
  if (home < 0) home = phases.length - 1;
  if (unmatched.length > 0) {
    if (home < 0) {
      phases.push({ label: "", name: "Tasks", slots: [], todo: null, added: [] });
      home = 0;
    }
    phases[home].added = unmatched;
  }

  const started = (p: Phase) =>
    (p.todo != null && p.todo.status !== "pending") || p.slots.some((s) => taskStateOf(s) !== "pending");
  // Criteria-only phases get a row for the build itself; without a live todo, finished checks or a later
  // phase starting mean it was built.
  const implStates: (TaskState | null)[] = phases.map((p, i) => {
    if (mode === "draft" || i >= doc.phases.length || p.slots.some((s) => !s.check)) return null;
    if (p.todo) {
      const state = todoState(p.todo);
      if (state !== "pending") return state;
    }
    const checksDone = p.slots.length > 0 && p.slots.every((s) => taskStateOf(s) === "done");
    return status === "implemented" || checksDone || phases.slice(i + 1).some(started) ? "done" : "pending";
  });

  const tracked: TrackedPhase[] = phases.map((p, i) => {
    const impl = implStates[i];
    const tasks: TrackedTask[] = [
      ...p.slots.map((s) => ({
        key: s.key,
        title: s.title,
        state: taskStateOf(s),
        added: false,
        check: s.check,
        group: s.group,
        reason: null,
        jump: jumpOf(s.todo) ?? jumpOf(p.todo),
      })),
      ...p.added.map((todo, i) => ({
        key: `added:${p.label}.${i + 1}`,
        title: todo.subject,
        state: todoState(todo),
        added: true,
        check: false,
        group: null,
        reason: null,
        jump: jumpOf(todo),
      })),
      // Last, so slot and added rows keep their indices.
      ...(impl
        ? [
            {
              key: `impl:${p.label}`,
              title: "Implementation",
              state: impl,
              added: false,
              check: false,
              group: null,
              reason: null,
              jump: jumpOf(p.todo),
            },
          ]
        : []),
    ];
    return { label: p.label, name: p.name, state: phaseStateOf(p, tasks.map((t) => t.state)), tasks };
  });

  let current: CurrentItem | null = null;
  let currentTask: TrackedTask | null = null;
  let currentIndex = -1;
  for (const todo of todos) {
    if (todo.status !== "in_progress") continue;
    for (const [i, p] of phases.entries()) {
      const slot = p.slots.findIndex((s) => s.todo === todo);
      const added = p.added.indexOf(todo);
      if (slot >= 0) currentTask = tracked[i].tasks[slot];
      else if (added >= 0) currentTask = tracked[i].tasks[p.slots.length + added];
      else if (p.todo !== todo) continue;
      const phaseLevel = slot < 0 && added < 0;
      if (phaseLevel && implStates[i]) currentTask = tracked[i].tasks[p.slots.length + p.added.length];
      currentIndex = i;
      current = {
        title: phaseLevel ? todo.subject : (currentTask?.title ?? todo.subject),
        state: "in_progress",
        reason: null,
        taskKey: currentTask && !currentTask.added ? currentTask.key : null,
        phaseLevel,
        subagent: session?.subagent ?? null,
      };
      break;
    }
    if (current) break;
  }

  const activity = session?.activity ?? "idle";
  if (current && (activity === "waiting" || activity === "failed")) {
    current.state = activity;
    current.reason = session?.reason ?? null;
    if (currentTask) {
      currentTask.state = activity;
      currentTask.reason = current.reason;
      if (activity === "failed" && session?.failureItemId) {
        currentTask.jump = { itemId: session.failureItemId };
      }
    }
  }

  const all = tracked.flatMap((p) => p.tasks);
  const total = all.length;
  const done = all.filter((t) => t.state === "done").length;
  const checks = all.filter((t) => t.check);
  // Phases stand in for tasks when no phase has any; a phase being built gives its pending rows half credit.
  const units: (TaskState | PhaseState)[] =
    total > 0
      ? tracked.flatMap((p, i) => {
          const building = phases[i].todo?.status === "in_progress";
          return p.tasks.map((t) => (building && t.state === "pending" ? "in_progress" : t.state));
        })
      : tracked.map((p) => p.state);
  const unitsDone = units.filter((s) => s === "done").length;
  const half = units.filter((s) => s === "in_progress" || s === "waiting").length;
  const allDone = units.length > 0 && unitsDone === units.length;
  if (allDone) current = null;
  const built = (p: Phase, i: number) =>
    p.todo?.status === "completed" ||
    implStates[i] === "done" ||
    (p.slots.some((s) => !s.check) && p.slots.every((s) => s.check || taskStateOf(s) === "done"));
  const tasksDone = allDone || (doc.phases.length > 0 && phases.slice(0, doc.phases.length).every(built));

  let currentPhase: number | null = null;
  if (tracked.length > 0) {
    const index = allDone ? -1 : current ? currentIndex : tracked.findIndex((p) => p.state !== "done");
    currentPhase = index >= 0 ? index + 1 : tracked.length;
  }

  return {
    mode,
    title: doc.title,
    status: doc.status,
    author: doc.author,
    overview: doc.overview,
    body: doc.body,
    phases: tracked,
    current,
    currentPhase,
    done,
    total,
    checks: { done: checks.filter((t) => t.state === "done").length, total: checks.length },
    progress: units.length > 0 ? (unitsDone + 0.5 * half) / units.length : 0,
    allDone,
    tasksDone,
    pulsing: activity === "working" && current?.state === "in_progress",
    writeBacks: slots
      .filter((s) => !s.checked && s.todo?.status === "completed")
      .map((s) => ({ key: s.key, title: s.title })),
  };
}
