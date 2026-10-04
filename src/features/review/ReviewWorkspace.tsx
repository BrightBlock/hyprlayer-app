import { useEffect, useMemo, useRef } from "react";

import { EmptyState, ErrorNote, Spinner, TabStrip } from "@/components/ui";
import { openReview, type ReviewTab } from "@/lib/appNav";
import type { ReviewComment, ReviewFile, ReviewNote, WorkstreamView } from "@/lib/bindings";
import { useMarkReviewFilesViewed, useReview, useSetReviewFileViewed } from "@/lib/queries";
import type { PlanDocPhase } from "@/features/plan/planDoc";
import { usePlanTracker } from "@/features/plan/usePlanTracker";
import { isBusy, isWorking, type TranscriptItem } from "@/features/session/transcript";

import { attributeFiles } from "./attribution";
import { DiffPane } from "./DiffPane";
import { FileTree, folderGroups, taskGroups } from "./FileTree";
import { PlanCoverage } from "./PlanCoverage";
import { ReviewHeader } from "./ReviewHeader";
import { selectReviewFile, setReviewFilter, setReviewGroupBy, useWorkstreamReview } from "./reviewStore";
import { groupThreads, openCountsByPath } from "./threads";
import { useImplementSession } from "./useImplementSession";
import { useReviewDelivery } from "./useReviewDelivery";

const NO_FILES: ReviewFile[] = [];
const NO_COMMENTS: ReviewComment[] = [];
const NO_NOTES: ReviewNote[] = [];
const NO_ITEMS: TranscriptItem[] = [];
const NO_PHASES: PlanDocPhase[] = [];

function nextUnviewed(ordered: ReviewFile[], from: number, viewed: ReadonlySet<string>): ReviewFile | null {
  for (let step = 1; step < ordered.length; step++) {
    const f = ordered[(from + step) % ordered.length];
    if (!viewed.has(f.path)) return f;
  }
  return null;
}

export function ReviewWorkspace({
  view,
  tab,
  file,
  line,
  onOpenDoc,
}: {
  view: WorkstreamView;
  tab: ReviewTab;
  file?: string;
  line?: number;
  onOpenDoc: (path: string) => void;
}) {
  const wsId = view.item.id;
  const review = useReview(wsId);
  const { session, entry, agentLabel } = useImplementSession(wsId);
  const { key: planKey, plan, tracker, presentation, error: planError } = usePlanTracker(session?.id);
  const state = useWorkstreamReview(wsId);
  const setViewed = useSetReviewFileViewed();
  const markAll = useMarkReviewFilesViewed();
  const delivery = useReviewDelivery(wsId, session, agentLabel);
  const agentWorking = entry ? isWorking(entry) : (session?.working ?? false);
  const agentBusy = entry ? isBusy(entry) : (session?.working ?? false);

  // A linked file wins on the render it arrives, so the first diff fetched is the one asked for.
  const appliedFile = useRef<string | undefined>(undefined);
  const linked = file && appliedFile.current !== file ? file : null;
  useEffect(() => {
    if (!file) {
      appliedFile.current = undefined;
      return;
    }
    appliedFile.current = file;
    selectReviewFile(wsId, file);
  }, [wsId, file]);

  const files = review.data?.files.files ?? NO_FILES;
  const viewedPaths = review.data?.viewed;
  const viewed = useMemo(() => new Set(viewedPaths ?? []), [viewedPaths]);
  const items = entry?.items ?? NO_ITEMS;
  const phases = plan?.phases ?? NO_PHASES;
  const since = presentation?.toolUseId ?? null;
  const worktreePath = view.item.worktreePath;
  const attribution = useMemo(
    () => attributeFiles(items, phases, worktreePath, since),
    [items, phases, worktreePath, since],
  );
  const byTask = state.groupBy === "task" && attribution.tracked;
  const groups = useMemo(
    () => (byTask ? taskGroups(files, attribution, phases) : folderGroups(files)),
    [byTask, files, attribution, phases],
  );
  const ordered = useMemo(() => groups.flatMap((g) => g.files), [groups]);
  const changedPaths = review.data?.changedSinceRound;
  const changed = useMemo(() => {
    const touched = new Set(changedPaths ?? []);
    return touched.size > 0 ? files.filter((f) => touched.has(f.path)) : NO_FILES;
  }, [files, changedPaths]);
  const latestRound = review.data?.rounds[review.data.rounds.length - 1];
  const comments = review.data?.comments ?? NO_COMMENTS;
  const notes = review.data?.notes ?? NO_NOTES;
  const threads = useMemo(() => groupThreads(comments), [comments]);
  const counts = useMemo(() => openCountsByPath(threads, notes), [threads, notes]);

  const wanted = linked ?? state.selected;
  const index = Math.max(0, ordered.findIndex((f) => f.path === wanted));
  const current = ordered[index] ?? null;
  const next =
    (current && nextUnviewed(ordered, index, viewed)) ??
    (ordered.length > 1 ? ordered[(index + 1) % ordered.length] : null);
  const allViewed = ordered.length > 0 && ordered.every((f) => viewed.has(f.path));
  const fileThreads = useMemo(
    () => threads.filter((t) => t.root.path === current?.path),
    [threads, current?.path],
  );
  const fileNotes = useMemo(() => notes.filter((n) => n.path === current?.path), [notes, current?.path]);

  const markViewed = (f: ReviewFile, value: boolean) =>
    setViewed.mutate({ workstreamId: wsId, path: f.path, contentOid: f.contentOid, viewed: value });
  const select = (path: string) => selectReviewFile(wsId, path);
  const planTab = tab === "plan" && planKey !== null;
  const showTab = (next: ReviewTab) =>
    openReview({ projectPath: view.item.projectPath, workstreamId: wsId, tab: next });
  const step = (delta: 1 | -1) => {
    if (ordered.length === 0) return;
    select(ordered[(index + delta + ordered.length) % ordered.length].path);
  };

  return (
    <div className="flex min-w-0 flex-1 flex-col bg-surface">
      <ReviewHeader
        view={view}
        review={review.data}
        files={review.data?.files}
        viewedCount={ordered.filter((f) => viewed.has(f.path)).length}
        markingAll={markAll.isPending}
        onMarkAllViewed={() =>
          markAll.mutate({ workstreamId: wsId, files: ordered.filter((f) => !viewed.has(f.path)) })
        }
        planKey={planKey}
        agentLabel={agentLabel}
        agentWorking={agentWorking}
        agentBusy={agentBusy}
        delivering={delivery.delivering}
        onResend={() => {
          if (latestRound?.message) void delivery.deliver(latestRound, latestRound.message);
        }}
        onRequested={(result) => {
          if (result.message) void delivery.deliver(result.round, result.message);
        }}
        onOpenDoc={onOpenDoc}
      />
      <TabStrip
        tabs={[
          {
            key: "files",
            label: (
              <>
                Files <span className="tabular-nums text-ink-dim">{ordered.length}</span>
              </>
            ),
          },
          ...(planKey !== null
            ? [
                {
                  key: "plan" as const,
                  label: (
                    <>
                      Plan{" "}
                      {tracker && (
                        <span className="tabular-nums text-ink-dim">
                          {tracker.done}/{tracker.total}
                        </span>
                      )}
                    </>
                  ),
                },
              ]
            : []),
        ]}
        value={planTab ? "plan" : "files"}
        onChange={showTab}
        className="bg-panel px-2.5"
      />
      {review.isLoading ? (
        <div className="flex flex-1 items-center justify-center">
          <Spinner />
        </div>
      ) : review.isError ? (
        <div className="p-4">
          <ErrorNote error={review.error} />
        </div>
      ) : planTab ? (
        plan ? (
          <PlanCoverage
            plan={plan}
            files={files}
            attribution={attribution}
            viewed={viewed}
            onSelectFile={(path) => {
              select(path);
              showTab("files");
            }}
          />
        ) : planError ? (
          <div className="p-4">
            <ErrorNote error={planError} />
          </div>
        ) : (
          <div className="flex flex-1 items-center justify-center">
            <Spinner />
          </div>
        )
      ) : ordered.length === 0 ? (
        <EmptyState title="No changes yet" />
      ) : (
        <div className="flex min-h-0 flex-1">
          <aside className="w-[288px] shrink-0 border-r border-edge bg-panel">
            <FileTree
              groups={groups}
              changed={changed}
              selected={current?.path ?? null}
              viewed={viewed}
              counts={counts}
              filter={state.filter}
              groupBy={attribution.tracked ? state.groupBy : null}
              onFilter={(value) => setReviewFilter(wsId, value)}
              onGroupBy={(value) => setReviewGroupBy(wsId, value)}
              onSelect={select}
              onToggleViewed={(f) => markViewed(f, !viewed.has(f.path))}
            />
          </aside>
          {current && (
            <DiffPane
              view={view}
              file={current}
              index={index}
              total={ordered.length}
              viewed={viewed.has(current.path)}
              next={next}
              allViewed={allViewed}
              line={current.path === file ? line : undefined}
              threads={fileThreads}
              notes={fileNotes}
              agentLabel={agentLabel}
              onToggleViewed={() => markViewed(current, !viewed.has(current.path))}
              onPrev={() => step(-1)}
              onNext={() => step(1)}
              onMarkViewedNext={() => {
                if (!viewed.has(current.path)) markViewed(current, true);
                if (next) select(next.path);
              }}
            />
          )}
        </div>
      )}
    </div>
  );
}
