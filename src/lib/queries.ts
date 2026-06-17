import { keepPreviousData, useMutation, useQuery } from "@tanstack/react-query";

import {
  commands,
  type BinaryKind,
  type CreateDocInput,
  type FmPatchDto,
  type ThoughtQuery,
  type WorkstreamView,
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

/** Refresh everything a project add/remove touches: the sidebar list,
 *  per-project detail, the doc index, and the cross-project workstream tree
 *  (a removed project drops its workstreams from the sidebar). */
function invalidateProjects() {
  void queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
  void queryClient.invalidateQueries({ queryKey: ["project"] });
  void queryClient.invalidateQueries({ queryKey: ["docs"] });
  void queryClient.invalidateQueries({ queryKey: ["workstreams"] });
}

/** Import a project from a local git repo on disk (cloned into the repos root). */
export function useAddProject() {
  return useMutation({
    mutationFn: (args: { path: string; directoryName: string | null; bootstrapUser: string | null }) =>
      unwrap(commands.addProject(args.path, args.directoryName, args.bootstrapUser)),
    onSettled: invalidateProjects,
  });
}

/** Clone a hosted repo (a GitHub URL) into the repos root and register it. */
export function useCloneProject() {
  return useMutation({
    mutationFn: (args: {
      url: string;
      location: string | null;
      name: string | null;
      bootstrapUser: string | null;
    }) => unwrap(commands.cloneProject(args.url, args.location, args.name, args.bootstrapUser)),
    onSettled: invalidateProjects,
  });
}

export function useRemoveProject() {
  return useMutation({
    mutationFn: (args: { path: string; deleteFiles: boolean }) =>
      unwrap(commands.removeProject(args.path, args.deleteFiles)),
    onSettled: invalidateProjects,
  });
}

/** The user's GitHub repos for the clone dialog (empty when gh is unavailable). */
export function useRecentRepos(enabled = true) {
  return useQuery({
    queryKey: ["recent-repos"],
    queryFn: () => unwrap(commands.listRecentRepos()),
    enabled,
    staleTime: 5 * 60_000,
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

/* ------------------------------- Workstreams ------------------------------ */
// List cache lives at ["workstreams", projectPath]; everything scoped to one
// item (docs, diff, detail) lives under the ["workstream", id] prefix so a
// single prefix invalidation refreshes the whole family.

function invalidateWorkstream(id: string) {
  // Whole ["workstreams"] prefix: the project's list AND the sidebar's
  // cross-project tree both refresh.
  void queryClient.invalidateQueries({ queryKey: ["workstreams"] });
  void queryClient.invalidateQueries({ queryKey: ["workstream", id] });
}

export function useWorkstreams(projectPath: string) {
  return useQuery({
    queryKey: ["workstreams", projectPath],
    queryFn: () => unwrap(commands.listWorkstreams(projectPath, false)),
  });
}

/** Active workstreams across all projects — the sidebar tree. */
export function useAllWorkstreams(enabled = true) {
  return useQuery({
    queryKey: ["workstreams", "@all"],
    queryFn: () => unwrap(commands.listAllWorkstreams()),
    enabled,
  });
}

export function useCreateWorkstream(projectPath: string) {
  return useMutation({
    mutationFn: (title: string) => unwrap(commands.createWorkstream(projectPath, title)),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["workstreams"] });
    },
  });
}

export function useRenameWorkstream() {
  return useMutation({
    mutationFn: ({ id, title }: { id: string; title: string }) =>
      unwrap(commands.renameWorkstream(id, title)),
    onSettled: (_data, _error, { id }) => invalidateWorkstream(id),
  });
}

export function useSetWorkstreamStage(projectPath: string) {
  return useMutation({
    mutationFn: ({ id, stage }: { id: string; stage: string }) =>
      unwrap(commands.setWorkstreamStage(id, stage)),
    // Optimistic: the stage drives the composer styling, so flip it instantly.
    onMutate: async ({ id, stage }) => {
      await queryClient.cancelQueries({ queryKey: ["workstreams", projectPath] });
      const previous = queryClient.getQueriesData<WorkstreamView[]>({
        queryKey: ["workstreams", projectPath],
      });
      queryClient.setQueriesData<WorkstreamView[]>(
        { queryKey: ["workstreams", projectPath] },
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
    // Only this project's list needs the refetch safety net: a stage change
    // alters nothing the sidebar tree or the diff family render, so don't
    // trigger the cross-project git status fan-out for every mode click.
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ["workstreams", projectPath] });
    },
  });
}

export function useArchiveWorkstream() {
  return useMutation({
    mutationFn: ({ id, discard }: { id: string; discard: boolean }) =>
      unwrap(commands.archiveWorkstream(id, discard)),
    onSuccess: (_data, { id }) => invalidateWorkstream(id),
  });
}

export function useReprovisionWorkstream() {
  return useMutation({
    mutationFn: (id: string) => unwrap(commands.reprovisionWorkstream(id)),
    onSettled: (_data, _error, id) => invalidateWorkstream(id),
  });
}

// ── Source-control panel ──────────────────────────────────────────────────

/** Invalidate the things a staging action changes: the split status and the
 *  workstream's own view (its dirty/line counts in the sidebar tree). */
function invalidateWorktreeGit(id: string) {
  void queryClient.invalidateQueries({ queryKey: ["workstream", id, "git-status"] });
  void queryClient.invalidateQueries({ queryKey: ["workstream", id, "file-sides"] });
  invalidateWorkstream(id);
}

export function useWorktreeGitStatus(id: string, enabled = true) {
  return useQuery({
    queryKey: ["workstream", id, "git-status"],
    queryFn: () => unwrap(commands.worktreeGitStatus(id)),
    enabled: enabled && id.length > 0,
    // No keepPreviousData: it would show another workstream's files while the
    // new one's query is in-flight. Same-key refetches keep data visible by
    // default, so the refresh spinner still works.
  });
}

/** Two sides of a file for the merge-view diff. Keyed by (id, path, staged) so
 *  the staged and working diffs of the same file cache separately. */
export function useFileSides(id: string, path: string, staged: boolean) {
  return useQuery({
    queryKey: ["workstream", id, "file-sides", path, staged],
    queryFn: () => unwrap(commands.worktreeFileSides(id, path, staged)),
    enabled: id.length > 0 && path.length > 0,
  });
}

/** One level of the "All Files" tree. Each expanded directory is its own query. */
export function useWorktreeDir(id: string, relPath: string, enabled = true) {
  return useQuery({
    queryKey: ["workstream", id, "dir", relPath],
    queryFn: () => unwrap(commands.worktreeListDir(id, relPath)),
    enabled: enabled && id.length > 0,
  });
}

/** Contents of a worktree file for the read-only viewer tab. */
export function useWorktreeFile(id: string, path: string) {
  return useQuery({
    queryKey: ["workstream", id, "file", path],
    queryFn: () => unwrap(commands.worktreeReadFile(id, path)),
    enabled: id.length > 0 && path.length > 0,
  });
}

export function useStageFiles(id: string) {
  return useMutation({
    mutationFn: (paths: string[]) => unwrap(commands.worktreeStage(id, paths)),
    onSettled: () => invalidateWorktreeGit(id),
  });
}

export function useUnstageFiles(id: string) {
  return useMutation({
    mutationFn: (paths: string[]) => unwrap(commands.worktreeUnstage(id, paths)),
    onSettled: () => invalidateWorktreeGit(id),
  });
}

export function useDiscardFiles(id: string) {
  return useMutation({
    mutationFn: (paths: string[]) => unwrap(commands.worktreeDiscard(id, paths)),
    onSettled: () => invalidateWorktreeGit(id),
  });
}
