import type { KeyboardEvent, MouseEvent } from "react";

import { ActivityDot } from "@/components/ActivityDot";
import { Badge } from "@/components/ui";
import type { WorkstreamView } from "@/lib/bindings";
import { HARNESS_SHORT_LABEL } from "@/lib/domain/harnessing";
import { isDetachedHead, isFolderWorkstream, isOffBase, shortBranch } from "@/lib/domain/projects";
import { stageLabel } from "@/lib/domain/stages";
import { formatCompactAge, formatCompactCount } from "@/lib/format";
import { useSessionStore } from "@/features/session/sessionStore";
import { columnDot, pipelineColumnOf, type PipelineLive, type PipelineSession } from "./pipeline";
import { usePlanProgress, usePrSummary } from "./pipelineData";

export type PipelineCardHandlers = {
  onOpen: (view: WorkstreamView, sessionId: string | null) => void;
  onContextMenu?: (view: WorkstreamView, x: number, y: number) => void;
};

export function workstreamPlace(view: WorkstreamView): string {
  if (isFolderWorkstream(view)) return "folder";
  if (isDetachedHead(view)) return "detached HEAD";
  return shortBranch(view.item.branch);
}

export function harnessLabel(session: PipelineSession | undefined): string | null {
  return session ? HARNESS_SHORT_LABEL[session.harness] : null;
}

export function useAwaitingSince(live: PipelineLive | undefined): number | null {
  return useSessionStore((s) =>
    live?.activity === "needs-you" && live.sessionId ? (s.sessions[live.sessionId]?.awaitingSince ?? null) : null,
  );
}

export function cardEvents(view: WorkstreamView, live: PipelineLive | undefined, handlers: PipelineCardHandlers) {
  const open = () => handlers.onOpen(view, live?.sessionId ?? null);
  return {
    onClick: open,
    onKeyDown: (e: KeyboardEvent) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        open();
      }
    },
    onContextMenu:
      handlers.onContextMenu &&
      ((e: MouseEvent) => {
        e.preventDefault();
        handlers.onContextMenu?.(view, e.clientX, e.clientY);
      }),
  };
}

export function PipelineCard({
  view,
  live,
  session,
  now,
  handlers,
}: {
  view: WorkstreamView;
  live?: PipelineLive;
  session?: PipelineSession;
  now: number;
  handlers: PipelineCardHandlers;
}) {
  const { item, git } = view;
  const waiting = live?.activity === "needs-you";
  const column = pipelineColumnOf(view, waiting);
  const progress = usePlanProgress(item.id, column === "implementing");
  const pr = usePrSummary(view, column === "review");
  const since = useAwaitingSince(live);
  const inplace = item.worktreeMode === "inplace";
  const offBase = inplace && !isDetachedHead(view) && isOffBase(view);
  const percent = progress ? Math.round((progress.done / progress.total) * 100) : null;
  const harness = harnessLabel(session);

  return (
    <div
      role="button"
      tabIndex={0}
      aria-label={item.title}
      {...cardEvents(view, live, handlers)}
      title={inplace ? `${item.title} · main checkout (base: ${shortBranch(item.baseBranch)})` : item.title}
      className={`block w-full cursor-pointer rounded-card border px-3 py-[11px] text-left shadow-sm transition-colors ${
        waiting ? "border-warn/40 bg-q-chip-bg/40 hover:border-warn/70" : "border-edge bg-panel hover:border-ink-dim/50"
      }`}
    >
      <span className="flex items-center gap-2">
        {live && !waiting ? (
          <ActivityDot activity={live.activity} />
        ) : (
          column !== "planning" && <span className={`h-[7px] w-[7px] shrink-0 rounded-full ${columnDot(column)}`} />
        )}
        <span className="min-w-0 flex-1 truncate text-body font-semibold text-ink">{item.title}</span>
      </span>
      <span className="mt-1 flex min-w-0 items-center gap-1.5">
        <span className="min-w-0 truncate font-mono text-meta text-ink-4">
          {[workstreamPlace(view), harness].filter(Boolean).join(" · ")}
        </span>
        {inplace && <Badge tone="dim">no worktree</Badge>}
        {offBase && <Badge tone="dim">off base</Badge>}
      </span>
      {waiting && live?.line && <span className="mt-1.5 block text-ui-sm text-ink-dim">{live.line}</span>}
      {percent !== null && (
        <span className="mt-2.5 block h-1 overflow-hidden rounded-full bg-panel-2">
          <span className="block h-full bg-accent" style={{ width: `${percent}%` }} />
        </span>
      )}
      <span className="mt-2 flex min-w-0 items-center gap-2">
        {waiting ? (
          <span className="rounded-control bg-q-chip-bg px-1.5 py-0.5 font-mono text-meta font-semibold text-q-chip-text">
            {since !== null ? `Waited ${formatCompactAge(since, now)}` : "Waiting"}
          </span>
        ) : (
          <>
            {git.additions > 0 && (
              <span className="font-mono text-meta text-diff-add">+{formatCompactCount(git.additions)}</span>
            )}
            {git.deletions > 0 && (
              <span className="font-mono text-meta text-diff-del">−{formatCompactCount(git.deletions)}</span>
            )}
            <span className="min-w-0 truncate font-mono text-meta text-ink-dim">
              {pr ?? live?.line ?? (column === "planning" ? stageLabel(item.stage) : null)}
            </span>
          </>
        )}
        <span className="ml-auto shrink-0 font-mono text-meta text-ink-4">
          {waiting ? (
            <span className="font-sans font-semibold text-accent">Answer</span>
          ) : percent !== null ? (
            `${percent}%`
          ) : (
            session?.model
          )}
        </span>
      </span>
    </div>
  );
}
