import { cx } from "@/components/ui";

import type { JumpTarget, PhaseState, TaskState, TrackedPhase, TrackedTask } from "./planTracker";

const PHASE_SUFFIX: Record<PhaseState, string | null> = {
  done: "· done",
  in_progress: "· in progress",
  pending: null,
};

const BOX: Record<TaskState, string> = {
  done: "bg-ok text-accent-ink",
  in_progress: "border-[1.5px] border-ink",
  pending: "border-[1.5px] border-edge",
  waiting: "border-[1.5px] border-warn",
  failed: "border-[1.5px] border-bad",
};

const TEXT: Record<TaskState, string> = {
  done: "text-ink-4 line-through",
  in_progress: "font-semibold text-ink",
  pending: "text-ink-4",
  waiting: "text-ink-2",
  failed: "text-ink-2",
};

export function taskAriaLabel(task: TrackedTask): string {
  const state =
    task.state === "in_progress"
      ? "in progress"
      : task.state === "waiting"
        ? `waiting on you${task.reason ? `: ${task.reason}` : ""}`
        : task.state === "failed"
          ? `failed${task.reason ? `: ${task.reason}` : ""}`
          : task.state;
  return `${task.title}, ${state}${task.added ? ", added by the agent" : ""}`;
}

function TaskBox({ state, pulse }: { state: TaskState; pulse: boolean }) {
  return (
    <span
      data-box
      aria-hidden="true"
      className={cx(
        "relative mt-px flex size-4 shrink-0 items-center justify-center rounded-[4px]",
        BOX[state],
      )}
    >
      {state === "done" && (
        <svg width={10} height={10} viewBox="0 0 24 24" fill="none">
          <path
            d="m5 12.5 4.5 4.5L19 6.5"
            stroke="currentColor"
            strokeWidth={3}
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      )}
      {state === "in_progress" && (
        <span className={cx("absolute inset-[1.5px] rounded-[1.5px] bg-ink", pulse && "animate-plan-pulse")} />
      )}
    </span>
  );
}

// A draft is read-only, so its rows are not tab stops.
function DraftTaskRow({ task }: { task: TrackedTask }) {
  const state = task.state === "done" ? "done" : "pending";
  return (
    <div data-state={state} className="flex items-start gap-[9px] text-body">
      <TaskBox state={state} pulse={false} />
      <span className={cx("min-w-0 flex-1 break-words", state === "done" ? TEXT.done : "text-ink-2")}>
        {task.title}
      </span>
    </div>
  );
}

export function TaskRow({
  task,
  pulse,
  onJump,
}: {
  task: TrackedTask;
  pulse: boolean;
  onJump: (target: JumpTarget) => void;
}) {
  const active = task.state === "in_progress";
  return (
    <button
      type="button"
      data-state={task.state}
      data-current={active || task.state === "waiting" || task.state === "failed" ? "true" : undefined}
      aria-label={taskAriaLabel(task)}
      onClick={() => task.jump && onJump(task.jump)}
      className={cx(
        "flex w-full items-start gap-[9px] text-left text-body",
        active && "-mx-2 w-[calc(100%+16px)] rounded-control bg-panel-2 px-2 py-1.5",
      )}
    >
      <TaskBox state={task.state} pulse={pulse && active} />
      <span className="min-w-0 flex-1 break-words">
        <span className={TEXT[task.state]}>{task.title}</span>
        {task.added && (
          <span className="ml-1.5 rounded-control border border-edge px-1 align-[1px] text-meta text-ink-4">
            added
          </span>
        )}
        {task.reason && (task.state === "waiting" || task.state === "failed") && (
          <span
            className={cx(
              "mt-0.5 block text-meta",
              task.state === "waiting" ? "text-warn-ink" : "text-bad-ink",
            )}
          >
            {task.reason}
          </span>
        )}
      </span>
    </button>
  );
}

export function PhaseSection({
  phase,
  showState,
  pulse,
  flash = false,
  onJump,
}: {
  phase: TrackedPhase;
  showState: boolean;
  pulse: boolean;
  flash?: boolean;
  onJump: (target: JumpTarget) => void;
}) {
  const suffix = showState ? PHASE_SUFFIX[phase.state] : null;
  const title = phase.label ? `Phase ${phase.label} — ${phase.name}` : phase.name;
  return (
    <section className={cx("-mx-2 rounded-control px-2", flash && "animate-plan-flash")}>
      <div
        data-phase-label
        className={cx(
          "mb-2 text-label",
          showState && phase.state === "done" ? "text-ok" : "text-ink-4",
        )}
      >
        <span>{title}</span>
        {suffix && <span className="text-ink-4"> {suffix}</span>}
      </div>
      <ul className="space-y-2">
        {phase.tasks.map((task) => (
          <li key={task.key}>
            {showState ? <TaskRow task={task} pulse={pulse} onJump={onJump} /> : <DraftTaskRow task={task} />}
          </li>
        ))}
      </ul>
    </section>
  );
}
