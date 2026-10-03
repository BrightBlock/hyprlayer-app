import { useEffect, useRef, useState } from "react";

import { ErrorNote, Spinner } from "@/components/ui";
import { useSession } from "@/features/session/sessionStore";
import { isBusy, isWorking } from "@/features/session/transcript";
import { requestTranscriptJump } from "@/features/session/transcriptJump";
import type { ShellTab } from "@/features/session/workstreamTabs";
import { repoSummary } from "@/features/statusbar/statusBarState";
import type { WorkstreamView } from "@/lib/bindings";

import { PlanPanel } from "./PlanPanel";
import { approvalFor, runApproval } from "./planApproval";
import { checkOffTasks } from "./planDocActions";
import { closePlanPreview } from "./planPreviewStore";
import { reviewChecklistComponent } from "./reviewChecklistSlot";
import { usePlanTracker } from "./usePlanTracker";

export function PlanAside({
  view,
  shell,
  onOpenDoc,
  onOpenFile,
  onReviewChanges,
}: {
  view: WorkstreamView;
  shell: ShellTab;
  onOpenDoc: (path: string) => void;
  onOpenFile: (path: string) => void;
  onReviewChanges: () => void;
}) {
  const sessionId = shell.sessionId;
  const entry = useSession(sessionId);
  const { key, tracker, presentation, inline, indexed, loading, error, changed } = usePlanTracker(sessionId);
  const [approving, setApproving] = useState(false);
  const [approveError, setApproveError] = useState<unknown>(null);

  if (!key) return null;
  if (!tracker) {
    // The index can trail a fresh write by a moment; while the agent works that is not an error yet.
    const pending = loading || (entry ? isWorking(entry) : false);
    return (
      <div className="flex h-full items-center justify-center p-4">
        {pending ? <Spinner /> : <ErrorNote error={error ?? "Plan not found"} />}
      </div>
    );
  }

  const { action, blocked } = approvalFor({
    key,
    indexed,
    status: tracker.status,
    presentation,
    // Busy, not just working: a plan edit waiting on a permission prompt is still mid-write.
    busy: entry ? isBusy(entry) : false,
    pendingPermissionId: entry?.pendingPermission?.requestId ?? null,
    pendingQuestion: Boolean(entry?.pendingQuestion),
  });
  const presented = presentation?.toolUseId;
  const headerAction = inline
    ? presented && sessionId
      ? { label: "Show in chat", icon: "chat" as const, onClick: () => requestTranscriptJump(sessionId, { toolUseId: presented }) }
      : null
    : indexed
      ? { label: "Open in Library", icon: "external" as const, onClick: () => onOpenDoc(key) }
      : { label: "Open file", icon: "external" as const, onClick: () => onOpenFile(key) };
  const Checklist = tracker.tasksDone ? reviewChecklistComponent() : null;
  const panel = (
    <PlanPanel
      tracker={tracker}
      diff={repoSummary(view)}
      approving={approving}
      approve={action || blocked ? { blocked } : null}
      changed={changed}
      onClose={closePlanPreview}
      headerAction={headerAction}
      onReviewChanges={onReviewChanges}
      onJump={(target) => sessionId && requestTranscriptJump(sessionId, target)}
      onApprove={() => {
        if (!action || !sessionId) return;
        setApproving(true);
        setApproveError(null);
        runApproval(action, {
          sessionId,
          workstreamId: view.item.id,
          shell,
          key,
          pendingQuestion: entry?.pendingQuestion ?? null,
        })
          .catch(setApproveError)
          .finally(() => setApproving(false));
      }}
    />
  );

  return (
    <div className="flex h-full min-h-0 flex-col">
      {approveError != null && (
        <div className="shrink-0 p-2">
          <ErrorNote error={approveError} onDismiss={() => setApproveError(null)} />
        </div>
      )}
      <div className="min-h-0 flex-1">{Checklist ? <Checklist view={view} fallback={panel} /> : panel}</div>
    </div>
  );
}

// Writes `[x]` for tasks a live agent completed; tasks already complete when this mounted are left alone.
export function PlanDocSync({ sessionId }: { sessionId: string }) {
  const entry = useSession(sessionId);
  const { key: planKey, indexed, tracker } = usePlanTracker(sessionId);
  // Only a thoughts doc takes `[x]` write-backs; Claude's and inline plans are not ours to edit.
  const path = indexed ? planKey : null;
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
