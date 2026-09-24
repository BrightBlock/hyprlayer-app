import { todoPhaseLabel, type TimelineTask } from "@/features/session/planTasks";
import type { TranscriptJump } from "@/features/session/transcriptJump";

import type { PlanDoc } from "./planDoc";

export type JumpTarget = TranscriptJump;
export type TaskState = "done" | "in_progress" | "pending" | "waiting" | "failed";
export type PhaseState = "done" | "in_progress" | "pending";

export type TrackedTask = {
  key: string;
  title: string;
  state: TaskState;
  added: boolean;
  reason: string | null;
  jump: JumpTarget | null;
};

export type TrackedPhase = { label: string; name: string; state: PhaseState; tasks: TrackedTask[] };

export type CurrentItem = {
  title: string;
  state: "in_progress" | "waiting" | "failed";
  reason: string | null;
  taskKey: string | null;
};

export type SessionSignal = {
  stage: string;
  activity: "working" | "waiting" | "failed" | "idle";
  reason: string | null;
  failureItemId: string | null;
};

export type PlanTracker = {
  mode: "draft" | "tracking";
  title: string;
  status: string | null;
  author: string | null;
  overview: string;
  phases: TrackedPhase[];
  current: CurrentItem | null;
  currentPhase: number | null;
  done: number;
  total: number;
  progress: number;
  allDone: boolean;
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

type Slot = { key: string; title: string; checked: boolean; todo: TimelineTask | null };
type Phase = { label: string; name: string; slots: Slot[]; todo: TimelineTask | null; added: TimelineTask[] };

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

// Structure comes only from the doc; todos contribute status, and unmatched ones are appended as `added`.
export function trackPlan(
  doc: PlanDoc,
  todos: TimelineTask[],
  session: SessionSignal | null,
): PlanTracker {
  const phases: Phase[] = doc.phases.map((p) => ({
    label: p.label,
    name: p.name,
    slots: p.tasks.map((t) => ({ key: t.key, title: t.title, checked: t.checked, todo: null })),
    todo: null,
    added: [],
  }));
  const slots = phases.flatMap((p) => p.slots);
  const unmatched: TimelineTask[] = [];

  const phaseByLabel = (label: string) => phases.find((p) => p.label.toLowerCase() === label.toLowerCase());
  for (const todo of todos) {
    const tag = TASK_TAG.exec(todo.subject);
    const tagged = tag && phaseByLabel(tag[1])?.slots.find((s) => s.key.toLowerCase() === `p${tag[1]}.${tag[2]}`.toLowerCase());
    if (tagged && !tagged.todo) {
      tagged.todo = todo;
      continue;
    }
    const label = todoPhaseLabel(todo.subject);
    const phase = label !== null ? phaseByLabel(label) : undefined;
    if (phase) {
      phase.todo = todo;
      continue;
    }
    const wanted = normalize(todo.subject);
    const slot = slots.find((s) => !s.todo && normalize(s.title) === wanted);
    if (slot) slot.todo = todo;
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

  const tracked: TrackedPhase[] = phases.map((p) => {
    const tasks: TrackedTask[] = [
      ...p.slots.map((s) => ({
        key: s.key,
        title: s.title,
        state: taskStateOf(s),
        added: false,
        reason: null,
        jump: jumpOf(s.todo) ?? jumpOf(p.todo),
      })),
      ...p.added.map((todo, i) => ({
        key: `added:${p.label}.${i + 1}`,
        title: todo.subject,
        state: todoState(todo),
        added: true,
        reason: null,
        jump: jumpOf(todo),
      })),
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
      currentIndex = i;
      current = {
        title: currentTask?.title ?? todo.subject,
        state: "in_progress",
        reason: null,
        taskKey: currentTask && !currentTask.added ? currentTask.key : null,
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
  const half = all.filter((t) => t.state === "in_progress" || t.state === "waiting").length;
  const allDone = total > 0 && done === total;
  if (allDone) current = null;

  let currentPhase: number | null = null;
  if (tracked.length > 0) {
    const index = allDone ? -1 : current ? currentIndex : tracked.findIndex((p) => p.state !== "done");
    currentPhase = index >= 0 ? index + 1 : tracked.length;
  }

  const status = doc.status?.toLowerCase() ?? null;
  return {
    mode:
      session?.stage === "implement" || status === "active" || status === "implemented"
        ? "tracking"
        : "draft",
    title: doc.title,
    status: doc.status,
    author: doc.author,
    overview: doc.overview,
    phases: tracked,
    current,
    currentPhase,
    done,
    total,
    progress: total > 0 ? (done + 0.5 * half) / total : 0,
    allDone,
    pulsing: activity === "working" && current?.state === "in_progress",
    writeBacks: slots
      .filter((s) => !s.checked && s.todo?.status === "completed")
      .map((s) => ({ key: s.key, title: s.title })),
  };
}
