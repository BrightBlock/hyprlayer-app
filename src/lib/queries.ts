import { useEffect, useRef } from "react";
import { keepPreviousData, useMutation, useQuery } from "@tanstack/react-query";

import {
  commands,
  type ActiveSessionView,
  type AppSettings,
  type BinaryKind,
  type Bootstrap,
  type BranchPrefix,
  type CliInstallMethodId,
  type CreateDocInput,
  type FmPatchDto,
  type CreateProjectRequest,
  type Host,
  type HostsFile,
  type MergeMethod,
  type Project,
  type ScriptKind,
  type ScriptSettings,
  type Stage,
  type WorktreeMode,
  type ThoughtQuery,
  type ThoughtsProfile,
  type ThoughtsSource,
  type WorkstreamView,
  type WorktreeFile,
} from "./bindings";
import { pendingModelSync, type ModelOption } from "./domain/models";
import { unwrap } from "./ipc";
import { queryClient } from "./queryClient";

const workstreamArchiveCleanups: Array<(id: string) => void> = [];

export function onWorkstreamArchived(cleanup: (id: string) => void): void {
  workstreamArchiveCleanups.push(cleanup);
}

export function useBootstrap() {
  return useQuery({
    queryKey: ["bootstrap"],
    queryFn: () => unwrap(commands.getBootstrap()),
  });
}

// The level read behind the connect/recovery surface: asked at mount, on window
// focus, and when `serverDisconnected` fires. Nothing polls.
export function useConnectionStatus() {
  return useQuery({
    queryKey: ["connection-status"],
    queryFn: () => unwrap(commands.connectionStatus()),
    staleTime: 0,
  });
}

export function useHosts() {
  return useQuery({
    queryKey: ["hosts"],
    queryFn: () => unwrap(commands.listHosts()),
    staleTime: Infinity,
  });
}

export function useSaveHosts() {
  return useMutation({
    mutationFn: (file: HostsFile) => unwrap(commands.saveHosts(file)),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["hosts"] });
    },
  });
}

export function useTestHostConnection() {
  return useMutation({
    mutationFn: (host: Host) => unwrap(commands.testHostConnection(host)),
  });
}

// One SSH round trip, asked only while the install offer is on screen — which
// is why it is not cached past the component that mounts it.
export function useHostProvisioning(host: Host) {
  return useQuery({
    queryKey: ["host-provisioning", host.destination],
    queryFn: () => unwrap(commands.inspectHostProvisioning(host)),
    staleTime: 0,
    gcTime: 0,
  });
}

export function useLocalServers(enabled = true) {
  return useQuery({
    queryKey: ["local-servers"],
    queryFn: () => unwrap(commands.listLocalServers()),
    enabled,
    staleTime: 0,
  });
}

export function useShellName() {
  return useQuery({
    queryKey: ["shell-name"],
    queryFn: () => unwrap(commands.defaultShellName()),
    staleTime: Infinity,
  });
}

export function useFullDiskAccess() {
  return useQuery({
    queryKey: ["full-disk-access"],
    queryFn: () => commands.fullDiskAccessStatus(),
    staleTime: Infinity,
  });
}

export function useEditorTargets() {
  return useQuery({
    queryKey: ["editor-targets"],
    queryFn: () => unwrap(commands.detectEditors()),
    staleTime: Infinity,
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
      void queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
    },
  });
}

export function useThoughtsGitStatus(path: string | undefined) {
  return useQuery({
    queryKey: ["thoughts-git-status", path],
    queryFn: () => unwrap(commands.getThoughtsGitStatus(path ?? "")),
    enabled: !!path,
    retry: false,
    refetchInterval: 30_000,
  });
}

export const MIN_DOC_TEXT_QUERY = 3;

export function useDocs(query: ThoughtQuery, enabled = true) {
  return useQuery({
    queryKey: ["docs", query],
    queryFn: () => unwrap(commands.queryDocs(query)),
    enabled,
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

export function useDocWebUrl(id: string) {
  return useQuery({
    queryKey: ["doc-web-url", id],
    queryFn: () => unwrap(commands.docWebUrl(id)),
    enabled: id.length > 0,
    staleTime: 5 * 60_000,
  });
}

export function invalidateBootstrap() {
  void queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
}

function invalidateProjects() {
  void queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
  void queryClient.invalidateQueries({ queryKey: ["project"] });
  void queryClient.invalidateQueries({ queryKey: ["docs"] });
  void queryClient.invalidateQueries({ queryKey: ["workstreams"] });
}

export function useRepoPathInfo(path: string, enabled = true) {
  return useQuery({
    queryKey: ["repo-path-info", path],
    queryFn: () => unwrap(commands.inspectRepoPath(path)),
    enabled: enabled && path.trim().length > 0,
    staleTime: 0,
  });
}

export function useImportProject() {
  return useMutation({
    mutationFn: (args: {
      path: string;
      branch: string | null;
      directoryName: string | null;
      thoughtsDir: string | null;
      profile: string | null;
      bootstrapUser: string | null;
    }) =>
      unwrap(
        commands.importProject(
          args.path,
          args.branch,
          args.directoryName,
          args.thoughtsDir,
          args.profile,
          args.bootstrapUser,
        ),
      ),
    onSettled: invalidateProjects,
  });
}

export function useImportLocalFolder() {
  return useMutation({
    mutationFn: (args: { path: string; directoryName: string | null }) =>
      unwrap(commands.importLocalFolder(args.path, args.directoryName)),
    onSettled: invalidateProjects,
  });
}

export function useCloneProject() {
  return useMutation({
    mutationFn: (args: {
      url: string;
      location: string | null;
      name: string | null;
      thoughtsDir: string | null;
      profile: string | null;
      bootstrapUser: string | null;
    }) =>
      unwrap(
        commands.cloneProject(
          args.url,
          args.location,
          args.name,
          args.thoughtsDir,
          args.profile,
          args.bootstrapUser,
        ),
      ),
    onSettled: invalidateProjects,
  });
}

export function useCreateProject() {
  return useMutation({
    mutationFn: (req: CreateProjectRequest) => unwrap(commands.createProject(req)),
    onSettled: invalidateProjects,
  });
}

// `unavailable` is thrown rather than returned so react-query retries it and
// never caches it as an answer — a transient `gh` failure that resolved to a
// value would read as "signed out" for the whole staleTime.
export function useGithubOwners(enabled = true) {
  return useQuery({
    queryKey: ["github-owners"],
    queryFn: async () => {
      const result = await unwrap(commands.listGithubOwners());
      if (result.state === "unavailable") {
        throw new Error(result.message);
      }
      return result.state === "ready" ? result.owners : [];
    },
    enabled,
    staleTime: 10 * 60_000,
  });
}

export function usePublishProjectToGithub() {
  return useMutation({
    mutationFn: (args: {
      opId: string;
      path: string;
      owner: string;
      name: string;
      isPrivate: boolean;
    }) =>
      unwrap(
        commands.publishProjectToGithub(
          args.opId,
          args.path,
          args.owner,
          args.name,
          args.isPrivate,
        ),
      ),
    onSettled: invalidateProjects,
  });
}

export function useThoughtsProfiles(enabled = true) {
  return useQuery({
    queryKey: ["thoughts-profiles"],
    queryFn: () => unwrap(commands.listThoughtsProfiles()),
    enabled,
    staleTime: 30_000,
  });
}

export function useCreateThoughtsProfile() {
  return useMutation({
    mutationFn: (args: { name: string; thoughtsRepo: string | null }) =>
      unwrap(commands.createThoughtsProfile(args.name, args.thoughtsRepo)),
    onSuccess: (created) => {
      queryClient.setQueryData<ThoughtsProfile[]>(["thoughts-profiles"], (prev) =>
        prev ? [...prev, created] : prev,
      );
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ["thoughts-profiles"] });
      void queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
    },
  });
}

export function useRemoveProject() {
  return useMutation({
    mutationFn: (args: { path: string; deleteFiles: boolean }) =>
      unwrap(commands.removeProject(args.path, args.deleteFiles)),
    onSettled: invalidateProjects,
  });
}

export function useReorderProjects() {
  return useMutation({
    scope: { id: "reorder-projects" },
    mutationFn: (paths: string[]) => unwrap(commands.reorderProjects(paths)),
    onMutate: async (paths) => {
      await queryClient.cancelQueries({ queryKey: ["bootstrap"] });
      const previous = queryClient.getQueryData<Bootstrap>(["bootstrap"]);
      queryClient.setQueryData<Bootstrap>(["bootstrap"], (prev) => {
        if (!prev) return prev;
        const byPath = new Map(prev.projects.map((p) => [p.path, p]));
        const ordered = paths
          .map((path) => byPath.get(path))
          .filter((p): p is Project => p !== undefined);
        for (const p of prev.projects) if (!paths.includes(p.path)) ordered.push(p);
        return { ...prev, projects: ordered };
      });
      return { previous };
    },
    onError: (_error, _paths, context) => {
      if (context?.previous) queryClient.setQueryData(["bootstrap"], context.previous);
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
    },
  });
}

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

export function useCliInstallMethods() {
  return useQuery({
    queryKey: ["cli-install-methods"],
    queryFn: () => unwrap(commands.detectCliInstallMethods()),
  });
}

export function useInstallHyprlayerCli() {
  return useMutation({
    mutationFn: (method: CliInstallMethodId) => unwrap(commands.installHyprlayerCli(method)),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
      void queryClient.invalidateQueries({ queryKey: ["health"] });
    },
  });
}

export function useInstallOpencode() {
  return useMutation({
    mutationFn: () => unwrap(commands.installOpencode()),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
      void queryClient.invalidateQueries({ queryKey: ["health"] });
    },
  });
}

export function useSetAppSettings() {
  return useMutation({
    scope: { id: "app-settings" },
    mutationFn: (settings: AppSettings) => unwrap(commands.setAppSettings(settings)),
    onMutate: async (settings) => {
      await queryClient.cancelQueries({ queryKey: ["bootstrap"] });
      const previous = queryClient.getQueryData<Bootstrap>(["bootstrap"]);
      queryClient.setQueryData<Bootstrap>(["bootstrap"], (prev) =>
        prev ? { ...prev, settings } : prev,
      );
      return { previous };
    },
    onError: (_error, _settings, context) => {
      if (context?.previous) queryClient.setQueryData(["bootstrap"], context.previous);
    },
    onSuccess: (next) => {
      queryClient.setQueryData<Bootstrap>(["bootstrap"], (prev) =>
        prev ? { ...prev, settings: next } : prev,
      );
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
      void queryClient.invalidateQueries({ queryKey: ["branchPrefixes"] });
    },
  });
}

export function useGithubUsername() {
  return useQuery({
    queryKey: ["github-username"],
    queryFn: () => unwrap(commands.githubUsername()),
    staleTime: 10 * 60_000,
  });
}

export function useDefaultUsername() {
  return useQuery({
    queryKey: ["default-username"],
    queryFn: () => commands.defaultUsername(),
    staleTime: Infinity,
  });
}


export function invalidateWorkstreamLists() {
  void queryClient.invalidateQueries({ queryKey: ["workstreams"] });
}

function invalidateWorkstream(id: string) {
  invalidateWorkstreamLists();
  void queryClient.invalidateQueries({ queryKey: ["workstream", id] });
}

export function useWorkstreams(projectPath: string) {
  return useQuery({
    queryKey: ["workstreams", projectPath],
    queryFn: () => unwrap(commands.listWorkstreams(projectPath, false)),
  });
}

export function useAllWorkstreams(enabled = true) {
  return useQuery({
    queryKey: ["workstreams", "@all"],
    queryFn: () => unwrap(commands.listAllWorkstreams()),
    enabled,
    staleTime: 30_000,
  });
}

export function useWorkstreamSessions(workstreamId: string, enabled = true) {
  return useQuery({
    queryKey: ["workstream-sessions", workstreamId],
    queryFn: () => unwrap(commands.listSessions(workstreamId)),
    enabled: enabled && Boolean(workstreamId),
  });
}

export function useActiveSessions(enabled = true) {
  return useQuery<ActiveSessionView[]>({
    queryKey: ["active-sessions"],
    queryFn: () => unwrap(commands.listActiveSessions()),
    enabled,
  });
}

export function useWorkstreamBases(projectPath: string, enabled = true) {
  return useQuery({
    queryKey: ["workstream-bases", projectPath],
    queryFn: () => unwrap(commands.listWorkstreamBases(projectPath)),
    enabled,
    staleTime: 0,
  });
}

export function useFetchProjectRemote(projectPath: string) {
  return useMutation({
    mutationFn: () => unwrap(commands.fetchProjectRemote(projectPath)),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["workstream-bases", projectPath] });
    },
  });
}


export function useCreateWorkstream(projectPath: string) {
  return useMutation({
    mutationFn: ({
      title,
      base,
      mode,
      branch,
      branchPrefix,
    }: {
      title: string;
      base: string | null;
      mode: WorktreeMode;
      branch: string | null;
      branchPrefix: string | null;
    }) => unwrap(commands.createWorkstream(projectPath, title, base, mode, branch, branchPrefix)),
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
    mutationFn: ({ id, stage }: { id: string; stage: Stage }) =>
      unwrap(commands.setWorkstreamStage(id, stage)),
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
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ["workstreams", projectPath] });
    },
  });
}

export function useArchiveWorkstream() {
  return useMutation({
    mutationFn: ({ id, skipScripts = false }: { id: string; skipScripts?: boolean }) =>
      unwrap(commands.archiveWorkstream(id, skipScripts)),
    onSuccess: (_data, { id }) => {
      for (const cleanup of workstreamArchiveCleanups) cleanup(id);
      invalidateWorkstream(id);
    },
  });
}

export function useArchivedWorkstreams(enabled = true) {
  return useQuery({
    queryKey: ["workstreams", "@archived"],
    queryFn: () => unwrap(commands.listArchivedWorkstreams()),
    enabled,
  });
}

export function useDiscardProjectChanges(projectPath: string) {
  return useMutation({
    mutationFn: () => unwrap(commands.discardProjectChanges(projectPath)),
  });
}

export function useRestoreWorkstream() {
  return useMutation({
    mutationFn: (id: string) => unwrap(commands.restoreWorkstream(id)),
    onSuccess: (_data, id) => invalidateWorkstream(id),
  });
}

export function useDeleteArchivedWorkstream() {
  return useMutation({
    mutationFn: (id: string) => unwrap(commands.deleteArchivedWorkstream(id)),
    onSuccess: (_data, id) => invalidateWorkstream(id),
  });
}

export function useReprovisionWorkstream() {
  return useMutation({
    mutationFn: (id: string) => unwrap(commands.reprovisionWorkstream(id)),
    onSettled: (_data, _error, id) => invalidateWorkstream(id),
  });
}

export function useCleanupArchivedWorkstreams() {
  return useMutation({
    mutationFn: (beforeMs: number | null) =>
      unwrap(commands.cleanupArchivedWorkstreams(beforeMs)),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["workstreams"] }),
  });
}


const PR_ERROR_RETRY_MS = 60_000;

export function invalidateWorkstreamPr(id: string) {
  void queryClient.invalidateQueries({ queryKey: ["workstream-pr", id] });
}

export function useWorkstreamPr(id: string, enabled = true, pollMs?: number) {
  return useQuery({
    queryKey: ["workstream-pr", id],
    queryFn: () => unwrap(commands.workstreamPrStatus(id)),
    enabled: enabled && id.length > 0,
    staleTime: 8000,
    refetchInterval: (query) => {
      if (query.state.status === "error") return PR_ERROR_RETRY_MS;
      return pollMs && !query.state.data ? pollMs : false;
    },
  });
}

export function useMergePr(id: string) {
  return useMutation({
    mutationFn: (method: MergeMethod) => unwrap(commands.mergeWorkstreamPr(id, method)),
    onSuccess: () => {
      invalidateWorkstreamPr(id);
      invalidateWorkstream(id);
    },
  });
}

export function useWorkstreamOpenPrs(id: string, query: string, enabled = true) {
  return useQuery({
    queryKey: ["workstream-pr", id, "open", query],
    queryFn: () => unwrap(commands.listWorkstreamOpenPrs(id, query)),
    enabled: enabled && id.length > 0,
    staleTime: 15_000,
  });
}

export function useWorkstreamPrPreview(id: string, prNumber: number | null, enabled = true) {
  return useQuery({
    queryKey: ["workstream-pr", id, "preview", prNumber],
    queryFn: () => unwrap(commands.previewWorkstreamPr(id, prNumber as number)),
    enabled: enabled && id.length > 0 && prNumber !== null,
    staleTime: 15_000,
  });
}

export function useAttachPr(id: string) {
  return useMutation({
    mutationFn: (prNumber: number) => unwrap(commands.attachWorkstreamPr(id, prNumber)),
    onSuccess: () => {
      invalidateWorkstreamPr(id);
      invalidateWorkstream(id);
    },
  });
}

export function useDetachPr() {
  return useMutation({
    mutationFn: (id: string) => unwrap(commands.detachWorkstreamPr(id)),
    onSuccess: (_data, id) => {
      invalidateWorkstreamPr(id);
      invalidateWorkstream(id);
    },
  });
}

export function useSetWorkstreamBase(id: string) {
  return useMutation({
    mutationFn: (base: string) => unwrap(commands.setWorkstreamBaseBranch(id, base)),
    onSuccess: () => {
      invalidateWorktreeGit(id);
      invalidateWorkstreamPr(id);
      invalidateWorkstream(id);
    },
  });
}

export function useUpdateFromBase(id: string) {
  return useMutation({
    mutationFn: () => unwrap(commands.updateWorkstreamFromBase(id)),
    onSuccess: () => {
      invalidateWorktreeGit(id);
      invalidateWorkstream(id);
    },
  });
}

export function useFetchWorkstreamRemote(id: string) {
  return useMutation({
    mutationFn: () => unwrap(commands.fetchWorkstreamRemote(id)),
    onSettled: () => {
      invalidateWorkstreamPr(id);
      invalidateWorktreeGit(id);
    },
  });
}


export function invalidateWorktreeGit(id: string) {
  void queryClient.invalidateQueries({ queryKey: ["workstream", id, "git-status"] });
  void queryClient.invalidateQueries({ queryKey: ["workstream", id, "file-sides"] });
  invalidateWorkstream(id);
}

export function useWorktreeGitStatus(id: string, enabled = true) {
  return useQuery({
    queryKey: ["workstream", id, "git-status"],
    queryFn: () => unwrap(commands.worktreeGitStatus(id)),
    enabled: enabled && id.length > 0,
  });
}

export function useFileSides(id: string, path: string, staged: boolean, enabled = true) {
  return useQuery({
    queryKey: ["workstream", id, "file-sides", path, staged],
    queryFn: () => unwrap(commands.worktreeFileSides(id, path, staged)),
    enabled: enabled && id.length > 0 && path.length > 0,
  });
}

export function useWorktreeDir(id: string, relPath: string, enabled = true) {
  return useQuery({
    queryKey: ["workstream", id, "dir", relPath],
    queryFn: () => unwrap(commands.worktreeListDir(id, relPath)),
    enabled: enabled && id.length > 0,
  });
}

export function useWorktreeFile(id: string, path: string) {
  return useQuery({
    queryKey: ["workstream", id, "file", path],
    queryFn: () => unwrap(commands.worktreeReadFile(id, path)),
    enabled: id.length > 0 && path.length > 0,
  });
}

export function useSaveWorktreeFile(id: string, path: string) {
  return useMutation({
    mutationFn: ({ content, baseMtimeMs }: { content: string; baseMtimeMs: number }) =>
      unwrap(commands.worktreeWriteFile(id, path, content, baseMtimeMs)),
    onSuccess: (mtimeMs, { content }) => {
      queryClient.setQueryData<WorktreeFile>(["workstream", id, "file", path], {
        content,
        mtimeMs,
      });
      invalidateWorktreeGit(id);
    },
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


export function useWorkstreamCommits(id: string, enabled = true) {
  return useQuery({
    queryKey: ["workstream", id, "commits"],
    queryFn: () => unwrap(commands.listWorkstreamCommits(id)),
    enabled: enabled && id.length > 0,
  });
}

export function useCommitFiles(id: string, sha: string, enabled = true) {
  return useQuery({
    queryKey: ["commit", id, sha, "files"],
    queryFn: () => unwrap(commands.worktreeCommitFiles(id, sha)),
    enabled: enabled && id.length > 0 && sha.length > 0,
    staleTime: Infinity,
  });
}

export function useCommitFileSides(id: string, sha: string, path: string) {
  return useQuery({
    queryKey: ["commit", id, sha, "sides", path],
    queryFn: () => unwrap(commands.worktreeCommitFileSides(id, sha, path)),
    enabled: id.length > 0 && sha.length > 0 && path.length > 0,
    staleTime: Infinity,
  });
}


export function useAiStatus() {
  return useQuery({
    queryKey: ["aiStatus"],
    queryFn: () => unwrap(commands.getAiStatus()),
    staleTime: Infinity,
  });
}

export function useAgentBundle(options: { enabled?: boolean } = {}) {
  return useQuery({
    queryKey: ["agent-bundle"],
    queryFn: () => unwrap(commands.agentBundleStatus()),
    staleTime: 5 * 60_000,
    enabled: options.enabled ?? true,
  });
}

export function useAgentBundleVersions(options: { enabled?: boolean } = {}) {
  return useQuery({
    queryKey: ["agent-bundle-versions"],
    queryFn: () => unwrap(commands.agentBundleVersions()),
    staleTime: 5 * 60_000,
    retry: false,
    enabled: options.enabled ?? true,
  });
}

export function invalidateCliBundleState() {
  void queryClient.invalidateQueries({ queryKey: ["aiStatus"] });
  void queryClient.invalidateQueries({ queryKey: ["agent-bundle"] });
  void queryClient.invalidateQueries({ queryKey: ["agent-bundle-versions"] });
  void queryClient.invalidateQueries({ queryKey: ["cli-update-check"] });
}

export function useCliUpdateCheck(options: { enabled?: boolean } = {}) {
  return useQuery({
    queryKey: ["cli-update-check"],
    queryFn: () => unwrap(commands.hyprlayerCliUpdateCheck()),
    staleTime: 5 * 60_000,
    retry: false,
    enabled: options.enabled ?? true,
  });
}

export function invalidateAgentBundle() {
  invalidateCliBundleState();
  void queryClient.invalidateQueries({ queryKey: ["agent-files"] });
  void queryClient.invalidateQueries({ queryKey: ["agent-skills"] });
}

export function useProbeDefaultThoughts() {
  return useQuery({
    queryKey: ["probe-default-thoughts"],
    queryFn: () => commands.probeDefaultThoughts(),
  });
}


export function useWorktreeFiles(path: string) {
  return useQuery({
    queryKey: ["worktreeFiles", path],
    queryFn: () => unwrap(commands.getWorktreeFiles(path)),
  });
}

export function useSetWorktreeFiles(path: string) {
  return useMutation({
    mutationFn: (patterns: string[]) => unwrap(commands.setWorktreeFiles(path, patterns)),
    onSuccess: (next) => {
      queryClient.setQueryData(["worktreeFiles", path], next);
      void queryClient.invalidateQueries({ queryKey: ["worktree-files-check", path] });
    },
  });
}

export function useWorktreeFilesCheck(path: string) {
  return useQuery({
    queryKey: ["worktree-files-check", path],
    queryFn: () => unwrap(commands.checkWorktreeFiles(path)),
  });
}


export function useBranchPrefixes(path: string, enabled = true) {
  return useQuery({
    queryKey: ["branchPrefixes", path],
    queryFn: () => unwrap(commands.getBranchPrefixes(path)),
    enabled,
  });
}

export function useSetBranchPrefixes(path: string) {
  return useMutation({
    mutationFn: (prefixes: BranchPrefix[]) => unwrap(commands.setBranchPrefixes(path, prefixes)),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["branchPrefixes", path] });
    },
  });
}


export function useScripts(path: string) {
  return useQuery({
    queryKey: ["scripts", path],
    queryFn: () => unwrap(commands.getScripts(path)),
  });
}

export function useSetScripts(path: string) {
  return useMutation({
    mutationFn: (scripts: ScriptSettings) => unwrap(commands.setScripts(path, scripts)),
    onSuccess: (_data, scripts) => {
      queryClient.setQueryData(["scripts", path], scripts);
    },
  });
}

export function useScriptRuntime(workstreamId: string) {
  return useQuery({
    queryKey: ["script-runtime", workstreamId],
    queryFn: () => unwrap(commands.getScriptRuntime(workstreamId)),
    enabled: !!workstreamId,
  });
}

export function invalidateScriptRuntime(workstreamId: string) {
  void queryClient.invalidateQueries({ queryKey: ["script-runtime", workstreamId] });
}

export function useScriptOutput(workstreamId: string, kind: ScriptKind, enabled = true) {
  return useQuery({
    queryKey: ["script-output", workstreamId, kind],
    queryFn: () => unwrap(commands.getScriptOutput(workstreamId, kind)),
    enabled: enabled && !!workstreamId,
  });
}

export function invalidateScriptOutput(workstreamId: string, kind: ScriptKind) {
  void queryClient.invalidateQueries({ queryKey: ["script-output", workstreamId, kind] });
}

export function forgetScriptOutput(workstreamId: string, kind: ScriptKind) {
  queryClient.setQueryData(["script-output", workstreamId, kind], null);
}


export function useOpencodeCatalog(enabled = true) {
  return useQuery({
    queryKey: ["opencode-catalog"],
    queryFn: () => unwrap(commands.opencodeCatalog(false)),
    enabled,
    staleTime: 60 * 60_000,
  });
}

export function useRefreshOpencodeCatalog() {
  return useMutation({
    mutationFn: () => unwrap(commands.opencodeCatalog(true)),
    onSuccess: (next) => queryClient.setQueryData(["opencode-catalog"], next),
  });
}

export function useOpencodeAuth() {
  return useQuery({
    queryKey: ["opencode-auth"],
    queryFn: () => unwrap(commands.opencodeAuthList()),
  });
}

export function useSetOpencodeProviderKey() {
  return useMutation({
    mutationFn: ({ provider, key }: { provider: string; key: string }) =>
      unwrap(commands.opencodeSetProviderKey(provider, key)),
    onSuccess: (next) => queryClient.setQueryData(["opencode-auth"], next),
  });
}

export function useRemoveOpencodeProvider() {
  return useMutation({
    mutationFn: (provider: string) => unwrap(commands.opencodeRemoveProvider(provider)),
    onSuccess: (next) => queryClient.setQueryData(["opencode-auth"], next),
  });
}

export function useSetOpencodeDefaultModel() {
  return useMutation({
    mutationFn: (model: string | null) => unwrap(commands.opencodeSetDefaultModel(model)),
    onSettled: () => queryClient.invalidateQueries({ queryKey: ["bootstrap"] }),
  });
}

export function useOllamaStatus(endpoint?: string | null, enabled = true) {
  return useQuery({
    queryKey: ["ollama-status", endpoint ?? null],
    queryFn: () => unwrap(commands.ollamaStatus()),
    enabled,
    staleTime: 15_000,
  });
}

export function useSetOllamaEndpoint() {
  return useMutation({
    mutationFn: (endpoint: string | null) => unwrap(commands.ollamaSetEndpoint(endpoint)),
    onSuccess: (next, endpoint) =>
      queryClient.setQueryData(["ollama-status", endpoint ?? null], next),
    onSettled: () => queryClient.invalidateQueries({ queryKey: ["bootstrap"] }),
  });
}

export function useSetOllamaModels() {
  return useMutation({
    mutationFn: (models: string[]) => unwrap(commands.ollamaSetSelectedModels(models)),
    onSettled: () => queryClient.invalidateQueries({ queryKey: ["bootstrap"] }),
  });
}

export function useSetOllamaDefaultModel() {
  return useMutation({
    mutationFn: (model: string | null) => unwrap(commands.ollamaSetDefaultModel(model)),
    onSettled: () => queryClient.invalidateQueries({ queryKey: ["bootstrap"] }),
  });
}

export function useClaudeAuthStatus(enabled = true) {
  return useQuery({
    queryKey: ["claude-auth-status"],
    queryFn: () => unwrap(commands.claudeAuthStatus()),
    enabled,
    staleTime: 5 * 60_000,
  });
}

export function useCodexProbe(enabled = true) {
  return useQuery({
    queryKey: ["codex-probe"],
    queryFn: () => unwrap(commands.codexProbe()),
    enabled,
    staleTime: 5 * 60_000,
  });
}

export function useSyncClaudeModels(
  catalog: readonly ModelOption[],
  catalogSettled: boolean,
  catalogLive: boolean,
) {
  const settings = useBootstrap().data?.settings.providers?.claude;
  const synced = useRef<"none" | "static" | "live">("none");
  const hasCatalog = catalog.length > 0;

  useEffect(() => {
    if (!settings || !catalogSettled || !hasCatalog) return;
    if (synced.current === "live" || (synced.current === "static" && !catalogLive)) return;
    const sync = pendingModelSync(settings.visibleModels, settings.seenModelKeys, catalog);
    synced.current = catalogLive ? "live" : "static";
    if (!sync) return;
    void commands
      .claudeSyncModels(sync.add, sync.seen)
      .then(() => queryClient.invalidateQueries({ queryKey: ["bootstrap"] }))
      .catch(() => {});
  }, [settings, catalog, catalogSettled, catalogLive, hasCatalog]);
}

export function useClaudeProbe(enabled = true) {
  return useQuery({
    queryKey: ["claude-probe"],
    queryFn: () => unwrap(commands.claudeProbe()),
    enabled,
    staleTime: 5 * 60_000,
  });
}

export function useCodexLoginWithApiKey() {
  return useMutation({
    mutationFn: (apiKey: string) => unwrap(commands.codexLoginWithApiKey(apiKey)),
    onSuccess: (probe) => queryClient.setQueryData(["codex-probe"], probe),
  });
}

export function useCodexLogout() {
  return useMutation({
    mutationFn: () => unwrap(commands.codexLogout()),
    onSuccess: (probe) => queryClient.setQueryData(["codex-probe"], probe),
  });
}


export function useInitThoughtsRepo() {
  return useMutation({
    mutationFn: (args: { user: string; source: ThoughtsSource }) =>
      unwrap(commands.initThoughtsRepo(args.user, args.source)),
    onSuccess: (data) => {
      queryClient.setQueryData<Bootstrap>(["bootstrap"], data);
      invalidateProjects();
    },
  });
}
