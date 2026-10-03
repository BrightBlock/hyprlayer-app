import { create } from "zustand";

export type ReviewGroupBy = "task" | "folder";

export type WorkstreamReviewState = { selected: string | null; filter: string; groupBy: ReviewGroupBy };

const INITIAL: WorkstreamReviewState = { selected: null, filter: "", groupBy: "folder" };

// Survives the workspace unmounting, so "← Chat" and back lands on the same file.
export const useReviewStore = create<{ byWs: Record<string, WorkstreamReviewState>; submitRequest: string | null }>(
  () => ({ byWs: {}, submitRequest: null }),
);

function patch(workstreamId: string, next: Partial<WorkstreamReviewState>): void {
  useReviewStore.setState((s) => ({
    byWs: { ...s.byWs, [workstreamId]: { ...(s.byWs[workstreamId] ?? INITIAL), ...next } },
  }));
}

export function selectReviewFile(workstreamId: string, path: string): void {
  patch(workstreamId, { selected: path });
}

export function setReviewFilter(workstreamId: string, filter: string): void {
  patch(workstreamId, { filter });
}

export function setReviewGroupBy(workstreamId: string, groupBy: ReviewGroupBy): void {
  patch(workstreamId, { groupBy });
}

export function useWorkstreamReview(workstreamId: string): WorkstreamReviewState {
  return useReviewStore((s) => s.byWs[workstreamId] ?? INITIAL);
}

// The Ship dialog's "Submit first" asks the next mounted header to open its popover.
export function requestSubmitPopover(workstreamId: string): void {
  useReviewStore.setState({ submitRequest: workstreamId });
}

export function clearSubmitRequest(): void {
  useReviewStore.setState({ submitRequest: null });
}

export function useSubmitRequested(workstreamId: string): boolean {
  return useReviewStore((s) => s.submitRequest === workstreamId);
}
