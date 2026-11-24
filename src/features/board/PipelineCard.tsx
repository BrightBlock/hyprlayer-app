import type { KeyboardEvent, MouseEvent, ReactNode } from "react";

import { ActivityDot } from "@/components/ActivityDot";
import { Badge } from "@/components/ui";
import type { WorkstreamView } from "@/lib/bindings";
import { workstreamGoalLabel } from "@/lib/domain/goals";
import { HARNESS_SHORT_LABEL } from "@/lib/domain/harnessing";
import { isDetachedHead, isFolderWorkstream, isOffBase, shortBranch } from "@/lib/domain/projects";
import { stageLabel } from "@/lib/domain/stages";
import { formatCompactAge, formatCompactCount } from "@/lib/format";
import type { LeadAsk } from "@/features/lead/lead";
import { LeadMark } from "@/features/lead/LeadChip";
import { useSessionStore, useWorkstreamGoal } from "@/features/session/sessionStore";
import { columnDot, pipelineColumnOf, type PipelineLive, type PipelineSession } from "./pipeline";
import { usePlanProgress, usePrSummary } from "./pipelineData";

export type PipelineCardHandlers = {
  onOpen: (view: WorkstreamView, sessionId: string | null) => void;
  onContextMenu?: (view: WorkstreamView, x: number, y: number) => void;
};

export function workstreamPlace(view: WorkstreamView): string {
  if (isFolderWorkstream(view)) return "folder";
  if (isDetachedHead(view)) return "detached HEAD";
  return shortBranch(view.git.branch ?? view.item.branch);
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
  driven,
  ask,
  now,
  handlers,
  parentTitle,
  fold,
}: {
  view: WorkstreamView;
  live?: PipelineLive;
  session?: PipelineSession;
  driven: boolean;
  ask?: LeadAsk;
  now: number;
  handlers: PipelineCardHandlers;
  parentTitle?: string;
  fold?: ReactNode;
}) {
  const { item, git } = view;
  const waiting = ask !== undefined || live?.activity === "needs-you";
  const column = pipelineColumnOf(view, waiting);
  const progress = usePlanProgress(item.id, column === "implementing");
  const pr = usePrSummary(view, column === "review");
  const ownSince = useAwaitingSince(live);
  const askedSince = useSessionStore((s) => (ask ? (s.sessions[ask.sessionId]?.awaitingSince ?? null) : null));
  const since = ask ? askedSince : ownSince;
  const line = ask ? ask.question.questions[0]?.question : waiting ? live?.line : null;
  const frame = waiting
    ? "border-warn/40 bg-q-chip-bg/40 hover:border-warn/70"
    : "border-edge bg-panel hover:border-ink-dim/50";
  const inplace = item.worktreeMode === "inplace";
  const offBase = inplace && !isDetachedHead(view) && isOffBase(view);
  const percent = progress ? Math.round((progress.done / progress.total) * 100) : null;
  const harness = harnessLabel(session);
  const goal = useWorkstreamGoal(item.id);

  // The fold sits outside the role="button", so it stays reachable and doesn't open the card.
  return (
    <div className={`rounded-card border shadow-sm ${frame}`}>
      <div
        role="button"
        tabIndex={0}
        aria-label={item.title}
        {...cardEvents(view, live, handlers)}
        title={inplace ? `${item.title} · main checkout (base: ${shortBranch(item.baseBranch)})` : item.title}
        className="block w-full cursor-pointer rounded-card px-3 py-[11px] text-left"
      >
        <span className="flex items-center gap-2">
          {live && !waiting ? (
            <ActivityDot activity={live.activity} />
          ) : (
            column !== "planning" && <span className={`h-[7px] w-[7px] shrink-0 rounded-full ${columnDot(column)}`} />
          )}
          {driven && (
            <span title="Driven by the Lead" className="shrink-0 font-mono text-meta text-ink-2">
              ◎
            </span>
          )}
          <span className="min-w-0 flex-1 truncate text-body font-semibold text-ink">{item.title}</span>
          {goal && (
            <span data-testid="board-goal-badge" className="shrink-0">
              <Badge tone="dim">{workstreamGoalLabel(goal)}</Badge>
            </span>
          )}
        </span>
        <span className="mt-1 flex min-w-0 items-center gap-1.5">
          <span className="min-w-0 truncate font-mono text-meta text-ink-4">
            {[parentTitle && `↳ ${parentTitle}`, workstreamPlace(view), harness]
              .filter(Boolean)
              .join(" · ")}
          </span>
          {inplace && <Badge tone="dim">no worktree</Badge>}
          {offBase && <Badge tone="dim">off base</Badge>}
        </span>
        {line && <span className="mt-1.5 block text-ui-sm text-ink-dim">{line}</span>}
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
            {ask ? (
              <span className="text-ink-dim">answer below ↓</span>
            ) : waiting ? (
              <span className="font-sans font-semibold text-accent">Answer</span>
            ) : percent !== null ? (
              `${percent}%`
            ) : (
              session?.model
            )}
          </span>
        </span>
      </div>
      {fold}
    </div>
  );
}

export function LeadCard({
  view,
  live,
  now,
  handlers,
}: {
  view: WorkstreamView;
  live: PipelineLive;
  now: number;
  handlers: PipelineCardHandlers;
}) {
  const since = useAwaitingSince(live);
  const question = useSessionStore((s) =>
    live.sessionId ? (s.sessions[live.sessionId]?.pendingQuestion?.questions[0]?.question ?? null) : null,
  );
  const line = question ?? live.line;

  return (
    <div
      role="button"
      tabIndex={0}
      aria-label="Lead Engineer"
      {...cardEvents(view, live, handlers)}
      className="block w-full cursor-pointer rounded-card border border-warn/40 bg-q-chip-bg/40 px-3 py-[11px] text-left shadow-sm hover:border-warn/70"
    >
      <span className="flex items-center gap-2">
        <LeadMark className="h-4 w-4" />
        <span className="min-w-0 flex-1 truncate text-body font-semibold text-ink">Lead Engineer</span>
      </span>
      {line && <span className="mt-1.5 block text-ui-sm text-ink-dim">{line}</span>}
      <span className="mt-2 flex min-w-0 items-center gap-2">
        <span className="rounded-control bg-q-chip-bg px-1.5 py-0.5 font-mono text-meta font-semibold text-q-chip-text">
          {since !== null ? `Waited ${formatCompactAge(since, now)}` : "Waiting"}
        </span>
        <span className="ml-auto shrink-0 font-mono text-meta text-ink-dim">
          {question ? "answer below ↓" : <span className="font-sans font-semibold text-accent">Answer</span>}
        </span>
      </span>
    </div>
  );
}
