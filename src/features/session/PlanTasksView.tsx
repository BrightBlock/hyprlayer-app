
import { Spinner } from "@/components/ui";
import { Icon } from "@/components/Icon";
import { useUiStore } from "@/lib/uiStore";
import { useSession, useSessionPlanTasks } from "./sessionStore";
import { isWorking } from "./transcript";
import type { PlanTask } from "./planTasks";

function doneCount(tasks: PlanTask[]): number {
  return tasks.reduce((n, t) => (t.status === "completed" ? n + 1 : n), 0);
}

function StatusGlyph({ status, working }: { status: PlanTask["status"]; working: boolean }) {
  if (status === "in_progress") {
    return working ? (
      <Spinner size="xs" />
    ) : (
      <span aria-hidden="true" className="size-2.5 rounded-[3px] bg-accent" />
    );
  }
  if (status === "completed") {
    return <Icon name="check" size="xs" className="text-ok" />;
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
    <li className="flex items-start gap-2 px-1.5 py-1 text-ui">
      <span className="mt-0.5 flex size-3 shrink-0 items-center justify-center">
        <StatusGlyph status={task.status} working={working} />
      </span>
      <span className={`min-w-0 flex-1 break-words ${tone}`}>{task.subject}</span>
    </li>
  );
}

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
  const label = `${open ? "Hide" : "Show"} tasks, ${done}/${tasks.length} done${
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
          <Icon name={open ? "chevronDown" : "chevronRight"} size="xs" />
        </span>
        <Icon name="tasks" size="sm" className={active && working ? "animate-work-pulse" : undefined} />
        <span className="text-label">Tasks</span>
        <span className="ml-auto text-meta">
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
