import { useIsMutating, useMutation } from "@tanstack/react-query";
import { create } from "zustand";

import { bundleChipState, type BundleFailure, type BundleReport } from "@/features/agents/agents";
import { commands } from "@/lib/bindings";
import { errorMessage, unwrap } from "@/lib/ipc";
import { invalidateAgentBundle, useAgentBundle } from "@/lib/queries";

// Outlives the footer: switching section unmounts it while an update may still be running.
export const useBundleUpdateStore = create<{
  report: BundleReport | null;
  failure: BundleFailure | null;
  setReport: (report: BundleReport | null) => void;
  setFailure: (failure: BundleFailure | null) => void;
  clear: () => void;
}>((set) => ({
  report: null,
  failure: null,
  setReport: (report) => set({ report }),
  setFailure: (failure) => set({ failure }),
  clear: () => set({ report: null, failure: null }),
}));

const UPDATE_KEY = ["agent-bundle-update"];

export function useBundleUpdate() {
  const bundle = useAgentBundle();
  const update = useMutation({
    mutationKey: UPDATE_KEY,
    mutationFn: (_vars: { from: string | null; to: string | null }) =>
      unwrap(commands.updateAgentBundle()),
    onSuccess: (result, { from, to }) => {
      invalidateAgentBundle();
      useBundleUpdateStore.getState().setReport({ ...result, from, to });
    },
    onError: (e, { from }) => {
      invalidateAgentBundle();
      useBundleUpdateStore.getState().setFailure({ message: errorMessage(e), stillOn: from });
    },
  });
  const updating = useIsMutating({ mutationKey: UPDATE_KEY }) > 0;

  const chip = bundleChipState(bundle.data, { loading: bundle.isLoading, error: bundle.isError });

  const startUpdate = () =>
    update.mutate(
      chip.kind === "updateAvailable"
        ? { from: chip.from, to: chip.to }
        : { from: chip.kind === "upToDate" ? chip.label : null, to: null },
    );

  return {
    chip,
    checking: bundle.isFetching,
    refresh: () => void bundle.refetch(),
    updating,
    startUpdate,
  };
}
