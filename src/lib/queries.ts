import { keepPreviousData, useMutation, useQuery } from "@tanstack/react-query";

import {
  commands,
  type AppSettings,
  type BinaryKind,
  type CreateDocInput,
  type FmPatchDto,
  type ThoughtQuery,
  type WorkItemView,
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

/* ------------------------------- Work items ------------------------------ */
// List cache lives at ["workItems", projectPath]; everything scoped to one
// item (docs, diff, detail) lives under the ["workItem", id] prefix so a
// single prefix invalidation refreshes the whole family.

function invalidateWorkItem(projectPath: string, id: string) {
  void queryClient.invalidateQueries({ queryKey: ["workItems", projectPath] });
  void queryClient.invalidateQueries({ queryKey: ["workItem", id] });
}

export function useWorkItems(projectPath: string, includeArchived = false) {
  return useQuery({
    // includeArchived is part of the key; prefix invalidation on
    // ["workItems", projectPath] still hits both variants.
    queryKey: ["workItems", projectPath, { includeArchived }],
    queryFn: () => unwrap(commands.listWorkItems(projectPath, includeArchived)),
  });
}

export function useCreateWorkItem(projectPath: string) {
  return useMutation({
    mutationFn: (title: string) => unwrap(commands.createWorkItem(projectPath, title)),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["workItems", projectPath] });
    },
  });
}

export function useRenameWorkItem(projectPath: string) {
  return useMutation({
    mutationFn: ({ id, title }: { id: string; title: string }) =>
      unwrap(commands.renameWorkItem(id, title)),
    onSettled: (_data, _error, { id }) => invalidateWorkItem(projectPath, id),
  });
}

export function useSetWorkItemStage(projectPath: string) {
  return useMutation({
    mutationFn: ({ id, stage }: { id: string; stage: string }) =>
      unwrap(commands.setWorkItemStage(id, stage)),
    // Optimistic: the stage drives the composer styling, so flip it instantly.
    onMutate: async ({ id, stage }) => {
      await queryClient.cancelQueries({ queryKey: ["workItems", projectPath] });
      const previous = queryClient.getQueriesData<WorkItemView[]>({
        queryKey: ["workItems", projectPath],
      });
      queryClient.setQueriesData<WorkItemView[]>(
        { queryKey: ["workItems", projectPath] },
        (views) =>
          views?.map((view) =>
            view.item.id === id ? { ...view, item: { ...view.item, stage } } : view,
          ),
      );
      return { previous };
    },
    onError: (_error, _vars, context) => {
      for (const [key, data] of context?.previous ?? []) {
        queryClient.setQueryData(key, data);
      }
    },
    onSettled: (_data, _error, { id }) => invalidateWorkItem(projectPath, id),
  });
}

export function useArchiveWorkItem(projectPath: string) {
  return useMutation({
    mutationFn: ({ id, discard }: { id: string; discard: boolean }) =>
      unwrap(commands.archiveWorkItem(id, discard)),
    onSuccess: (_data, { id }) => invalidateWorkItem(projectPath, id),
  });
}

export function useReprovisionWorkItem(projectPath: string) {
  return useMutation({
    mutationFn: (id: string) => unwrap(commands.reprovisionWorkItem(id)),
    onSettled: (_data, _error, id) => invalidateWorkItem(projectPath, id),
  });
}

export function useWorkItemDocs(id: string) {
  return useQuery({
    queryKey: ["workItem", id, "docs"],
    queryFn: () => unwrap(commands.listWorkItemDocs(id)),
    enabled: id.length > 0,
  });
}

export function useAttachWorkItemDoc(id: string) {
  return useMutation({
    mutationFn: ({ docPath, role }: { docPath: string; role: string }) =>
      unwrap(commands.attachWorkItemDoc(id, docPath, role)),
    // The command returns the updated doc list — write it through directly.
    onSuccess: (docs) => {
      queryClient.setQueryData(["workItem", id, "docs"], docs);
    },
  });
}

export function useDetachWorkItemDoc(id: string) {
  return useMutation({
    mutationFn: (docPath: string) => unwrap(commands.detachWorkItemDoc(id, docPath)),
    onSuccess: (docs) => {
      queryClient.setQueryData(["workItem", id, "docs"], docs);
    },
  });
}

export function useWorktreeDiff(id: string, enabled = true) {
  return useQuery({
    queryKey: ["workItem", id, "diff"],
    queryFn: () => unwrap(commands.worktreeDiff(id)),
    enabled: enabled && id.length > 0,
    // Keep the previous file list on screen while a refetch resolves.
    placeholderData: keepPreviousData,
  });
}
