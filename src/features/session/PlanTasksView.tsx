/**
 * The agent's live plan, presented as {@link PlanTasksPanel}: a drawer docked
 * above the composer with a persistent handle bar (count + chevron) that slides
 * the task list open and shut. Open/collapsed/cleared state is owned by
 * `SessionChat`.
 */

import { Spinner } from "../../components/ui";
import { ChecklistIcon, ChevronDownIcon } from "../../components/icons";
import type { PlanTask } from "./planTasks";

/** Completed-task count for the "n/total" progress label. */
function doneCount(tasks: PlanTask[]): number {
  return tasks.reduce((n, t) => (t.status === "completed" ? n + 1 : n), 0);
}

/** Spinner while working, else a static marker; check when done, box when pending. */
function StatusGlyph({ status, working }: { status: PlanTask["status"]; working: boolean }) {
  if (status === "in_progress") {
    return working ? (
      <Spinner size={12} />
    ) : (
      <span aria-hidden="true" className="size-2.5 rounded-[3px] bg-accent" />
    );
  }
  if (status === "completed") {
    return (
      <svg aria-hidden="true" width={12} height={12} viewBox="0 0 24 24" fill="none" className="text-ok">
        <path
          d="m5 12.5 4.5 4.5L19 6.5"
          stroke="currentColor"
          strokeWidth={2.5}
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
    );
  }
  return <span aria-hidden="true" className="size-3 rounded-[3px] border border-ink-dim/50" />;
}

function TaskRow({ task, working }: { task: PlanTask; working: boolean }) {
  const tone =
    task.status === "completed"
      ? "text-ink-dim line-through decoration-ink-dim/40"
      : task.status === "in_progress"
        ? "text-ink font-medium"
        : "text-ink-dim";
  return (
    <li className="flex items-start gap-2 px-1.5 py-1 text-xs">
      <span className="mt-0.5 flex size-3 shrink-0 items-center justify-center">
        <StatusGlyph status={task.status} working={working} />
      </span>
      <span className={`min-w-0 flex-1 break-words ${tone}`}>{task.subject}</span>
    </li>
  );
}

/**
 * The task drawer — a tab anchored to the top of the composer (its parent must
 * be `relative`), floating up over the transcript so it never pushes layout. Its
 * fill is solid panel-2, the same surface as the composer it merges into. The
 * list slides open above the handle via a grid-rows `0fr↔1fr` fold, and a second
 * outer fold retracts the whole drawer on auto-clear. Animates by height only.
 */
export function PlanTasksPanel({
  tasks,
  working,
  collapsed,
  cleared,
  onToggle,
}: {
  tasks: PlanTask[];
  working: boolean;
  /** List folded down to just the handle bar. */
  collapsed: boolean;
  /** All tasks done + turn ended → the whole drawer retracts into the composer. */
  cleared: boolean;
  onToggle: () => void;
}) {
  if (tasks.length === 0) return null;
  const done = doneCount(tasks);
  const open = !collapsed && !cleared;
  const active = tasks.find((t) => t.status === "in_progress");
  const label = `${open ? "Hide" : "Show"} tasks — ${done}/${tasks.length} done${
    active ? ` · ${active.subject}` : ""
  }`;
  // Shared fold transition for the grids below. Not gated on reduced motion —
  // Windows maps that to the often-off "Animation effects" toggle, which would
  // otherwise snap the drawer instead of folding it.
  const fold = "transition-[grid-template-rows] duration-300 ease-[cubic-bezier(0.22,1,0.36,1)]";
  return (
    <div
      aria-hidden={cleared}
      // Outer fold: retracts the whole drawer (handle + list) when cleared.
      className={`task-toast absolute bottom-full left-6 z-20 grid w-fit max-w-[calc(100%-3rem)] ${fold} ${
        cleared ? "is-folded grid-rows-[0fr]" : "grid-rows-[1fr]"
      }`}
    >
      <div className="overflow-hidden">
        {/* Solid panel-2 — the exact same fill as the composer below, so they read as one surface. */}
        <div className="overflow-hidden rounded-t-xl bg-panel-2">
          {/* The list, folded open above the handle. */}
          <div className={`grid ${fold} ${open ? "grid-rows-[1fr]" : "grid-rows-[0fr]"}`}>
            <div className="min-h-0 overflow-hidden">
              <ul className="max-h-56 overflow-y-auto px-1.5 pt-2">
                {tasks.map((t, i) => (
                  <TaskRow key={t.id ?? `pending-${i}`} task={t} working={working} />
                ))}
              </ul>
            </div>
          </div>
          {/* Handle bar: the always-visible merged tab; click to toggle the list. */}
          <button
            type="button"
            onClick={onToggle}
            title={label}
            aria-label={label}
            aria-expanded={open}
            tabIndex={cleared ? -1 : undefined}
            className="flex w-full items-center gap-1.5 px-3 py-1.5 text-ink-dim hover:text-ink"
          >
            {/* Pulse only while genuinely working — an ended turn can leave an in_progress task. */}
            <ChecklistIcon size={14} className={active && working ? "animate-work-pulse" : undefined} />
            <span className="text-[11px] font-semibold uppercase tracking-wider text-ink-dim/70">
              Tasks
            </span>
            <span className="font-mono text-[11px] text-ink-dim/60">
              {done}/{tasks.length}
            </span>
            {/* Chevron points up when collapsed (pull up to open), down when open. */}
            <ChevronDownIcon
              size={15}
              className={`ml-auto transition-transform duration-300 ${open ? "" : "rotate-180"}`}
            />
          </button>
        </div>
      </div>
    </div>
  );
}
