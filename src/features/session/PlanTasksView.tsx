/**
 * The agent's live plan/to-do list, presented as {@link SidebarTasksPanel}: a
 * collapsible section in the right "changes" sidebar, docked directly under the
 * Git section. A header bar (checklist glyph + count + chevron) folds the task
 * list open and shut; the collapsed state is a persisted, sidebar-level
 * preference in `uiStore`, so it stays put as you switch agent tabs. The panel
 * tracks the sidebar's focused session and renders nothing until it has a plan.
 */

import { Spinner } from "@/components/ui";
import { ChecklistIcon, ChevronDownIcon, ChevronRightIcon } from "@/components/icons";
import { useUiStore } from "@/lib/uiStore";
import { useSession, useSessionPlanTasks } from "./sessionStore";
import { isWorking } from "./transcript";
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
 * The Tasks section of the changes sidebar — the focused session's live plan,
 * below the pinned Git + Changes block. Behaves like a VS Code accordion pane:
 * expanded it grows (`flex-1`) to sit directly under Changes and share the
 * vertical space, its list scrolling within; collapsed it drops to a bare `h-9`
 * header (chevron + checklist glyph + "TASKS" + n/total) that sinks to the
 * bottom under the space-filling Changes. Absent until the session has a plan;
 * collapse state is the shared, persisted `tasksCollapsed` so it survives tab
 * switches/restart.
 */
export function SidebarTasksPanel({ sessionId }: { sessionId: string | undefined }) {
  const tasks = useSessionPlanTasks(sessionId);
  const session = useSession(sessionId);
  const working = session ? isWorking(session) : false;
  const collapsed = useUiStore((s) => s.tasksCollapsed);
  const toggle = useUiStore((s) => s.toggleTasks);

  if (tasks.length === 0) return null;
  const done = doneCount(tasks);
  const open = !collapsed;
  const active = tasks.find((t) => t.status === "in_progress");
  const label = `${open ? "Hide" : "Show"} tasks — ${done}/${tasks.length} done${
    active ? ` · ${active.subject}` : ""
  }`;

  return (
    <div className={`flex flex-col border-t border-edge ${open ? "min-h-0 flex-1" : "shrink-0"}`}>
      <button
        type="button"
        onClick={toggle}
        title={label}
        aria-label={label}
        aria-expanded={open}
        className="flex h-9 shrink-0 items-center gap-2 px-3 text-left text-ink-dim hover:text-ink"
      >
        <span className="flex w-3 shrink-0 justify-center">
          {open ? <ChevronDownIcon size={12} /> : <ChevronRightIcon size={12} />}
        </span>
        {/* Pulse only while genuinely working — an ended turn can leave an in_progress task. */}
        <ChecklistIcon size={14} className={active && working ? "animate-work-pulse" : undefined} />
        <span className="text-xs font-semibold uppercase tracking-wide">Tasks</span>
        <span className="ml-auto font-mono text-xs">
          {done}/{tasks.length}
        </span>
      </button>
      {open && (
        <ul className="min-h-0 flex-1 overflow-y-auto border-t border-edge px-1.5 py-1">
          {tasks.map((t, i) => (
            <TaskRow key={t.id ?? `pending-${i}`} task={t} working={working} />
          ))}
        </ul>
      )}
    </div>
  );
}
