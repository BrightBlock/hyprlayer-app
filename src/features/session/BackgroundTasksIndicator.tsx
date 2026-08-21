/**
 * A compact footer indicator for live background shell tasks, sitting in the
 * composer toolbar next to the context meter. Renders nothing when no tasks are
 * running. While tasks run it shows a pulsing terminal icon with a count;
 * clicking opens an upward popover (mirroring `ContextMeter`) that lists each
 * task, spinner, label, elapsed timer, and a running/waiting hint, where a row
 * click jumps the transcript to the spawning `Bash` call.
 */

import { useEffect, useRef, useState } from "react";
import { Popover, Spinner } from "@/components/ui";
import { TerminalIcon } from "@/components/icons";
import { formatElapsed } from "@/lib/format";
import type { LiveBackgroundTask } from "./sessionStore";

export function BackgroundTasksIndicator({
  tasks,
  onJumpTo,
}: {
  tasks: LiveBackgroundTask[];
  onJumpTo: (toolUseId: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const [now, setNow] = useState(() => Date.now());
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Tick the elapsed counters every second while there are live tasks.
  useEffect(() => {
    if (tasks.length === 0) {
      if (intervalRef.current !== null) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
      return;
    }
    if (intervalRef.current === null) {
      intervalRef.current = setInterval(() => setNow(Date.now()), 1000);
    }
    return () => {
      if (intervalRef.current !== null) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
    };
  }, [tasks.length]);

  // Drop the popover automatically once the last task ends, so it can't linger
  // open over nothing.
  useEffect(() => {
    if (tasks.length === 0 && open) setOpen(false);
  }, [tasks.length, open]);

  if (tasks.length === 0) return null;

  const countLabel =
    tasks.length === 1 ? "1 background task" : `${tasks.length} background tasks`;

  return (
    <div className="relative">
      <button
        ref={triggerRef}
        type="button"
        onClick={() => setOpen((v) => !v)}
        title={countLabel}
        aria-label={countLabel}
        aria-expanded={open}
        className="flex h-8 items-center gap-1.5 rounded-md px-2 text-background hover:bg-edge/50"
      >
        <TerminalIcon size={14} className="animate-work-pulse" />
        <span className="font-mono text-[11px]">{tasks.length}</span>
      </button>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={triggerRef}
        side="top"
        align="end"
        className="w-72 p-1.5"
      >
        <div className="px-1.5 pb-1 pt-0.5 text-[10px] font-semibold uppercase tracking-wider text-ink-dim/60">
          {countLabel}
        </div>
        {tasks.map((task) => {
          const elapsed = task.startedAt !== null ? formatElapsed(now - task.startedAt) : null;
          const isAwaiting = task.status === "awaiting";

          return (
            <button
              key={task.toolUseId}
              type="button"
              onClick={() => {
                onJumpTo(task.toolUseId);
                setOpen(false);
              }}
              className="flex w-full items-center gap-2 rounded-md px-1.5 py-1.5 text-left text-xs hover:bg-edge/40"
              title="Jump to transcript"
            >
              <Spinner size={12} />
              <span className="flex-1 truncate font-mono text-ink">{task.label}</span>
              {elapsed !== null && <span className="shrink-0 text-ink-dim">{elapsed}</span>}
              <span className={`shrink-0 ${isAwaiting ? "text-warn" : "text-ink-dim"}`}>
                {isAwaiting ? "waiting…" : "running"}
              </span>
            </button>
          );
        })}
      </Popover>
    </div>
  );
}
