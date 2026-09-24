import { useEffect, useMemo } from "react";

import { planDocForSession } from "@/features/session/planLink";
import { planTaskTimeline } from "@/features/session/planTasks";
import { useSession, type SessionEntry } from "@/features/session/sessionStore";
import { describeState, isWorking } from "@/features/session/transcript";
import { parseToolInput } from "@/features/session/toolDisplay";
import { useWrittenDocs } from "@/features/session/writtenDocs";
import { basename } from "@/lib/path";
import { useDoc } from "@/lib/queries";

import { parsePlanDoc } from "./planDoc";
import { trackPlan, type PlanTracker, type SessionSignal } from "./planTracker";

// Reuses the pane header's status vocabulary so the tracker never contradicts it.
export function sessionSignalOf(entry: SessionEntry): SessionSignal {
  const label = describeState(entry.state, entry.streaming).label;
  if (entry.pendingPermission) {
    return {
      stage: entry.stage,
      activity: "waiting",
      reason: `${label} · ${entry.pendingPermission.toolName}`,
      failureItemId: null,
    };
  }
  if (entry.pendingQuestion) {
    const header = entry.pendingQuestion.questions[0]?.header;
    return {
      stage: entry.stage,
      activity: "waiting",
      reason: header ? `${label} · ${header}` : label,
      failureItemId: null,
    };
  }
  if (entry.state === "crashed" || entry.state === "spawn_failed") {
    const failure = entry.items.filter((i) => i.kind === "error" || i.kind === "exit").pop();
    return { stage: entry.stage, activity: "failed", reason: label, failureItemId: failure?.id ?? null };
  }
  return {
    stage: entry.stage,
    activity: isWorking(entry) ? "working" : "idle",
    reason: null,
    failureItemId: null,
  };
}

const WRITE_TOOLS = new Set(["Write", "Edit", "MultiEdit"]);

// The index watcher reports canonical thoughts paths, which miss a plan opened through a
// worktree symlink; a finished write in the transcript is the backstop.
function planWrites(items: SessionEntry["items"], path: string): number {
  const name = basename(path);
  let n = 0;
  for (const item of items) {
    if (item.kind !== "tool" || !item.result || !WRITE_TOOLS.has(item.name)) continue;
    const target = parseToolInput(item.input)?.file_path;
    if (typeof target === "string" && basename(target) === name) n++;
  }
  return n;
}

export type PlanTrackerResult = {
  path: string | null;
  tracker: PlanTracker | null;
  loading: boolean;
  error: unknown;
};

export function usePlanPath(sessionId: string | undefined): string | null {
  const entry = useSession(sessionId);
  const written = useWrittenDocs(entry?.workstreamId ?? null);
  const items = entry?.items;
  const stage = entry?.stage ?? "";
  return useMemo(() => (items ? planDocForSession(items, stage, written) : null), [items, stage, written]);
}

export function usePlanTracker(sessionId: string | undefined): PlanTrackerResult {
  const entry = useSession(sessionId);
  const path = usePlanPath(sessionId);
  const doc = useDoc(path ?? "");
  const content = doc.data?.content;
  const parsed = useMemo(
    () => (content !== undefined && path ? parsePlanDoc(content, path) : null),
    [content, path],
  );
  const items = entry?.items;
  const timeline = useMemo(() => (items ? planTaskTimeline(items) : []), [items]);
  const writes = useMemo(() => (items && path ? planWrites(items, path) : 0), [items, path]);
  const { refetch } = doc;
  useEffect(() => {
    if (writes > 0) void refetch();
  }, [writes, refetch]);
  const signal = entry ? sessionSignalOf(entry) : null;
  const signalKey = signal ? JSON.stringify(signal) : "";
  const tracker = useMemo(
    () => (parsed ? trackPlan(parsed, timeline, signal) : null),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- signal is keyed by value
    [parsed, timeline, signalKey],
  );
  return { path, tracker, loading: Boolean(path) && doc.isLoading, error: doc.error };
}
