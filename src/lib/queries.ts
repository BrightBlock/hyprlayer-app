import { keepPreviousData, useMutation, useQuery } from "@tanstack/react-query";

import {
  commands,
  type AppSettings,
  type BinaryKind,
  type CreateDocInput,
  type FmPatchDto,
  type ThoughtQuery,
} from "./bindings";
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
      void queryClient.invalidateQueries({ queryKey: ["thoughtsGitStatus", path] });
      // A sync can rewrite the hyprlayer config (e.g. orphan-mapping cleanup),
      // which the sidebar renders from; don't wait for the watcher poll.
      void queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
    },
  });
}

export function useDocs(query: ThoughtQuery, enabled = true) {
  return useQuery({
    queryKey: ["docs", query],
    queryFn: () => unwrap(commands.queryDocs(query)),
    enabled,
    // Keep the previous list on screen while a new filter/search resolves.
    placeholderData: keepPreviousData,
  });
}

export function useDoc(id: string) {
  return useQuery({
    queryKey: ["doc", id],
    queryFn: () => unwrap(commands.getDoc(id)),
    enabled: id.length > 0,
  });
}

export function useSaveDoc(id: string) {
  return useMutation({
    mutationFn: ({ content, baseMtimeMs }: { content: string; baseMtimeMs: number | null }) =>
      unwrap(commands.saveDoc(id, content, baseMtimeMs)),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["doc", id] });
      void queryClient.invalidateQueries({ queryKey: ["docs"] });
    },
  });
}

export function usePatchDocFrontmatter(id: string) {
  return useMutation({
    mutationFn: ({ patch, baseMtimeMs }: { patch: FmPatchDto; baseMtimeMs: number | null }) =>
      unwrap(commands.patchDocFrontmatter(id, patch, baseMtimeMs)),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["doc", id] });
      void queryClient.invalidateQueries({ queryKey: ["docs"] });
    },
  });
}

export function useCreateDoc() {
  return useMutation({
    mutationFn: (input: CreateDocInput) => unwrap(commands.createDoc(input)),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["docs"] });
    },
  });
}

export function useRevealDoc() {
  return useMutation({
    mutationFn: (id: string) => unwrap(commands.revealDoc(id)),
  });
}

export function useThoughtsGitStatus(projectPath: string) {
  return useQuery({
    queryKey: ["thoughtsGitStatus", projectPath],
    queryFn: () => unwrap(commands.getThoughtsGitStatus(projectPath)),
  });
}

export function useAddProject() {
  return useMutation({
    mutationFn: (args: { path: string; directoryName: string | null; bootstrapUser: string | null }) =>
      unwrap(commands.addProject(args.path, args.directoryName, args.bootstrapUser)),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
      void queryClient.invalidateQueries({ queryKey: ["project"] });
      void queryClient.invalidateQueries({ queryKey: ["docs"] });
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
