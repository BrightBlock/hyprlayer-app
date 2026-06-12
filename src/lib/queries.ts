import { useMutation, useQuery } from "@tanstack/react-query";

import { commands, type AppSettings, type BinaryKind } from "./bindings";
import { unwrap } from "./ipc";
import { queryClient } from "./queryClient";

export function useBootstrap() {
  return useQuery({
    queryKey: ["bootstrap"],
    queryFn: () => unwrap(commands.getBootstrap()),
  });
}

export function useProjectDetail(path: string) {
  return useQuery({
    queryKey: ["project", path],
    queryFn: () => unwrap(commands.getProject(path)),
  });
}

export function useBackendHealth(path: string) {
  return useQuery({
    queryKey: ["health", path],
    queryFn: () => unwrap(commands.runBackendHealth(path)),
  });
}

export function useSyncThoughts(path: string) {
  return useMutation({
    mutationFn: (message: string | null) => unwrap(commands.syncThoughts(path, message)),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ["project", path] });
      void queryClient.invalidateQueries({ queryKey: ["health", path] });
      // A sync can rewrite the hyprlayer config (e.g. orphan-mapping cleanup),
      // which the sidebar renders from; don't wait for the watcher poll.
      void queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
    },
  });
}

export function useSetBinaryOverride() {
  return useMutation({
    mutationFn: ({ kind, path }: { kind: BinaryKind; path: string | null }) =>
      unwrap(commands.setBinaryOverride(kind, path)),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
      void queryClient.invalidateQueries({ queryKey: ["health"] });
    },
  });
}

export function useRedetectBinaries() {
  return useMutation({
    mutationFn: () => unwrap(commands.redetectBinaries()),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
      void queryClient.invalidateQueries({ queryKey: ["health"] });
    },
  });
}

export function useSaveSettings() {
  return useMutation({
    mutationFn: (settings: AppSettings) => unwrap(commands.setAppSettings(settings)),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
    },
  });
}
