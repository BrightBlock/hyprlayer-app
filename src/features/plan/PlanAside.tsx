import { useEffect, useRef, useState } from "react";

import { ErrorNote, Spinner } from "@/components/ui";
import { useSession } from "@/features/session/sessionStore";
import { requestTranscriptJump } from "@/features/session/transcriptJump";
import type { ShellTab } from "@/features/session/workstreamTabs";
import { repoSummary } from "@/features/statusbar/statusBarState";
import type { WorkstreamView } from "@/lib/bindings";

import { PlanPanel } from "./PlanPanel";
import { approvePlan, checkOffTasks } from "./planDocActions";
import { usePlanTracker } from "./usePlanTracker";

export function PlanAside({
  view,
  shell,
  onOpenDoc,
  onReviewChanges,
}: {
  view: WorkstreamView;
  shell: ShellTab;
  onOpenDoc: (path: string) => void;
  onReviewChanges: () => void;
}) {
  const sessionId = shell.sessionId;
  const { path, tracker, loading, error } = usePlanTracker(sessionId);
  const [approving, setApproving] = useState(false);
  const [approveError, setApproveError] = useState<unknown>(null);

  if (!path) return null;
  if (!tracker) {
    return (
      <div className="flex h-full items-center justify-center p-4">
        {loading ? <Spinner /> : <ErrorNote error={error ?? "Plan not found"} />}
      </div>
    );
  }
  return (
    <div className="flex h-full min-h-0 flex-col">
      {approveError != null && (
        <div className="shrink-0 p-2">
          <ErrorNote error={approveError} onDismiss={() => setApproveError(null)} />
        </div>
      )}
      <div className="min-h-0 flex-1">
        <PlanPanel
          tracker={tracker}
          diff={repoSummary(view)}
          approving={approving}
          onOpenInLibrary={() => onOpenDoc(path)}
          onReviewChanges={onReviewChanges}
          onJump={(target) => sessionId && requestTranscriptJump(sessionId, target)}
          onApprove={() => {
            setApproving(true);
            setApproveError(null);
            approvePlan(view.item.id, path, shell)
              .catch(setApproveError)
              .finally(() => setApproving(false));
          }}
        />
      </div>
    </div>
  );
}

// Writes `[x]` for tasks a live agent completed; tasks already complete when this mounted are left alone.
export function PlanDocSync({ sessionId }: { sessionId: string }) {
  const entry = useSession(sessionId);
  const { path, tracker } = usePlanTracker(sessionId);
  const handled = useRef<Set<string> | null>(null);
  const live = Boolean(entry?.live);
  const writeBacks = tracker?.mode === "tracking" ? tracker.writeBacks : null;
  const key = writeBacks?.map((t) => `${t.key}\t${t.title}`).join("\n") ?? "";

  useEffect(() => {
    if (!path || !writeBacks) return;
    const ids = writeBacks.map((t) => `${t.key}\t${t.title}`);
    if (!handled.current) {
      handled.current = new Set(ids);
      return;
    }
    if (!live) return;
    const fresh = writeBacks.filter((_, i) => !handled.current!.has(ids[i]));
    if (fresh.length === 0) return;
    for (const id of ids) handled.current.add(id);
    void checkOffTasks(path, fresh).catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed on the write-back set
  }, [path, key, live]);

  return null;
}
