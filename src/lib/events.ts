import {
  events,
  type ActiveSessionView,
  type Goal,
  type GoalBrief,
  type GoalChanged,
  type GoalDetail,
  type Session,
} from "./bindings";
import { isLiveGoal } from "./domain/goals";
import { queryClient } from "./queryClient";
import { refreshOmarchyTheme } from "./theme";
import {
  invalidateCliBundleState,
  invalidateReview,
  invalidateScriptOutput,
  invalidateScriptRuntime,
  invalidateWorkstreamLists,
  invalidateWorkstreamPr,
  invalidateWorktreeGit,
  refreshProjectWorkstreams,
} from "./queries";

export type EventBridge = () => Array<Promise<() => void>>;

const featureBridges: EventBridge[] = [];

export function registerEventBridge(bridge: EventBridge): void {
  featureBridges.push(bridge);
}

const reconnectHandlers: Array<() => void> = [];

/** Returns the unsubscribe, for a handler that lives only as long as a mounted view. */
export function onServerReconnected(handler: () => void): () => void {
  reconnectHandlers.push(handler);
  return () => {
    const i = reconnectHandlers.indexOf(handler);
    if (i >= 0) reconnectHandlers.splice(i, 1);
  };
}

export function notifyServerReconnected(): void {
  for (const handler of [...reconnectHandlers]) handler();
}

const missedHandlers: Array<() => void> = [];

/** Runs when the connection lost runtime events: re-read what they would have changed. */
export function onEventsMissed(handler: () => void): void {
  missedHandlers.push(handler);
}

// The payload carries no `activeMs`/`updatedAt`, so a flip into or out of `waiting` re-reads the clock.
function goalMoved(goal: Goal, p: GoalChanged): boolean {
  return (
    goal.status !== p.status ||
    goal.iteration !== p.iteration ||
    goal.limits.maxIterations !== p.maxIterations ||
    (goal.step === "waiting") !== (p.step === "waiting")
  );
}

// A step-only change patches the cached goal; the rest re-read only this workstream's lists, so the
// several writes per iteration don't refetch every list.
function applyGoalChanged(p: GoalChanged): void {
  const patch = (g: Goal): Goal => (g.id === p.goalId ? { ...g, step: p.step, pauseReason: p.pauseReason } : g);
  for (const [key, list] of queryClient.getQueriesData<Goal[]>({ queryKey: ["goals", p.workstreamId] })) {
    if (!list) continue;
    const row = list.find((g) => g.id === p.goalId);
    if (row && !goalMoved(row, p)) queryClient.setQueryData<Goal[]>(key, list.map(patch));
    else void queryClient.invalidateQueries({ queryKey: key, exact: true });
  }
  const detailKey = ["goal", p.goalId];
  const detail = queryClient.getQueryData<GoalDetail>(detailKey);
  if (detail && !goalMoved(detail.goal, p)) {
    queryClient.setQueryData<GoalDetail>(detailKey, { ...detail, goal: patch(detail.goal) });
  } else if (detail) {
    void queryClient.invalidateQueries({ queryKey: detailKey, exact: true });
  }
  patchSessionBriefs(p);
}

// `Session.goal` is exactly the payload's fields, so the session lists are patched, never re-read.
function patchSessionBriefs(p: GoalChanged): void {
  const { goalId: id, status, step, pauseReason, iteration, maxIterations } = p;
  const brief: GoalBrief | null = isLiveGoal(p) ? { id, status, step, pauseReason, iteration, maxIterations } : null;
  const withBrief = (s: Session): Session => (s.id === p.sessionId ? { ...s, goal: brief } : s);
  queryClient.setQueryData<Session[]>(["workstream-sessions", p.workstreamId], (rows) =>
    rows?.some((s) => s.id === p.sessionId) ? rows.map(withBrief) : rows,
  );
  queryClient.setQueryData<ActiveSessionView[]>(["active-sessions"], (rows) =>
    rows?.some((r) => r.session.id === p.sessionId)
      ? rows.map((r) => (r.session.id === p.sessionId ? { ...r, session: withBrief(r.session) } : r))
      : rows,
  );
}

export function attachEventListeners(): () => void {
  const unlisteners: Promise<() => void>[] = [
    ...featureBridges.flatMap((bridge) => bridge()),
    events.configChanged.listen(() => {
      void queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
      void queryClient.invalidateQueries({ queryKey: ["project"] });
      void queryClient.invalidateQueries({ queryKey: ["thoughts-profiles"] });
    }),
    events.binariesChanged.listen(() => {
      void queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
      void queryClient.invalidateQueries({ queryKey: ["health"] });
      void queryClient.invalidateQueries({ queryKey: ["claude-probe"] });
      void queryClient.invalidateQueries({ queryKey: ["project"] });
      invalidateCliBundleState();
    }),
    // The edge event tells the UI to re-read the level state.
    events.serverDisconnected.listen(() => {
      void queryClient.invalidateQueries({ queryKey: ["connection-status"] });
    }),
    events.eventsMissed.listen(() => {
      for (const handler of missedHandlers) handler();
      void queryClient.invalidateQueries();
    }),
    events.accountChanged.listen(() => {
      void queryClient.invalidateQueries({ queryKey: ["account"] });
    }),
    events.omarchyThemeChanged.listen(() => {
      void refreshOmarchyTheme();
    }),
    events.prStatusChanged.listen((event) => {
      invalidateWorkstreamPr(event.payload.workstreamId);
      // Not on every status change: CI churn would recompute git status for every worktree.
      if (event.payload.observedPrChanged) invalidateWorkstreamLists();
    }),
    events.workstreamsChanged.listen((event) => void refreshProjectWorkstreams(event.payload.projectPath)),
    events.worktreeChanged.listen((event) => {
      invalidateWorktreeGit(event.payload.workstreamId);
    }),
    // Only mutations announce this; a read that did would refetch itself forever.
    events.reviewChanged.listen((event) => {
      invalidateReview(event.payload.workstreamId);
    }),
    events.workstreamBranchChanged.listen((event) => {
      invalidateWorkstreamLists();
      invalidateWorkstreamPr(event.payload.workstreamId);
    }),
    events.goalChanged.listen((event) => applyGoalChanged(event.payload)),
    events.sessionLifecycle.listen(() => {
      void queryClient.invalidateQueries({ queryKey: ["active-sessions"] });
      void queryClient.invalidateQueries({ queryKey: ["workstream-sessions"] });
    }),
    events.scriptRunChanged.listen((event) => {
      const { workstreamId, kind } = event.payload;
      invalidateScriptRuntime(workstreamId);
      invalidateScriptOutput(workstreamId, kind);
    }),
    events.thoughtsIndexUpdated.listen((event) => {
      void queryClient.invalidateQueries({ queryKey: ["docs"] });
      void queryClient.invalidateQueries({ queryKey: ["doc-counts"] });
      void queryClient.invalidateQueries({ queryKey: ["related"] });
      void queryClient.invalidateQueries({ queryKey: ["thoughts-git-status"] });
      const { ids } = event.payload;
      if (ids.length === 0) {
        void queryClient.invalidateQueries({ queryKey: ["doc"] });
      } else {
        for (const id of ids) {
          void queryClient.invalidateQueries({ queryKey: ["doc", id] });
        }
      }
    }),
    events.docStateChanged.listen((event) => {
      void queryClient.invalidateQueries({ queryKey: ["docs"] });
      void queryClient.invalidateQueries({ queryKey: ["doc-counts"] });
      // Read toggles leave the bytes alone: an open doc isn't re-read, only refetched on its next open.
      const { ids, change } = event.payload;
      for (const id of ids) {
        void queryClient.invalidateQueries({
          queryKey: ["doc", id],
          refetchType: change === "agent" ? "active" : "none",
        });
      }
    }),
  ];

  return () => {
    for (const p of unlisteners) {
      void p.then((unlisten) => unlisten());
    }
  };
}
