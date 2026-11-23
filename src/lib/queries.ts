import { useEffect, useRef } from "react";
import { keepPreviousData, skipToken, useInfiniteQuery, useMutation, useQuery } from "@tanstack/react-query";

import {
  commands,
  type ActiveSessionView,
  type AddReviewCommentRequest,
  type AppSettings,
  type BinaryKind,
  type Bootstrap,
  type BranchPrefix,
  type CliInstallMethodId,
  type CreateDocInput,
  type CreateProjectRequest,
  type DeriveGoalRequest,
  type DocContent,
  type FrontmatterEditDto,
  type GoalPatch,
  type Host,
  type HostsFile,
  type MergeMethod,
  type Project,
  type ScriptKind,
  type ScriptSettings,
  type SetReviewFileViewedRequest,
  type SkillScope,
  type SkillsShView,
  type SkillSourceInput,
  type SkillTarget,
  type StartGoalRequest,
  type SubmitReviewRequest,
  type Stage,
  type WorkstreamView,
  type WorktreeMode,
  type ThoughtQuery,
  type ThoughtsProfile,
  type ThoughtsSource,
  type WorktreeFile,
} from "./bindings";
import type { HostProbeResult } from "./domain/hosts";
import { pendingModelSync, type ModelOption } from "./domain/models";
import { unwrap } from "./ipc";
import { queryClient } from "./queryClient";

const workstreamArchiveCleanups: Array<(id: string) => void> = [];

export function onWorkstreamArchived(cleanup: (id: string) => void): () => void {
  workstreamArchiveCleanups.push(cleanup);
  return () => {
    const at = workstreamArchiveCleanups.indexOf(cleanup);
    if (at >= 0) workstreamArchiveCleanups.splice(at, 1);
  };
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
    onSuccess: (probe, host) => {
      queryClient.setQueryData<Record<string, HostProbeResult>>(["host-probes"], (prev) => ({
        ...prev,
        [host.destination]: {
          ok: probe.ok,
          code: probe.error?.code,
          release: probe.serverRelease,
          at: Date.now(),
        },
      }));
    },
  });
}

const NO_PROBES: Record<string, HostProbeResult> = {};

// Nothing fetches this: `useTestHostConnection` fills it, and it lasts the session.
export function useHostProbes(): Record<string, HostProbeResult> {
  return (
    useQuery<Record<string, HostProbeResult>>({
      queryKey: ["host-probes"],
      queryFn: skipToken,
      gcTime: Infinity,
    }).data ?? NO_PROBES
  );
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

export function useBrowserTargets() {
  return useQuery({
    queryKey: ["browser-targets"],
    queryFn: () => unwrap(commands.detectBrowsers()),
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
    // An unconfigured repo errors by design; focus and the post-sync invalidate still recover.
    refetchInterval: (query) => (query.state.error ? false : 30_000),
  });
}

export const MIN_DOC_TEXT_QUERY = 3;

export function useDocs(query: ThoughtQuery, enabled = true) {
  return useQuery({
    queryKey: ["docs", query],
    queryFn: () => unwrap(commands.queryDocs(query)),
    enabled,
    // A disabled query fetches nothing, so carrying data over would show a stale search.
    placeholderData: enabled ? keepPreviousData : undefined,
  });
}

// Attached, agent-written and branch-named docs; under ["docs"] so index changes refetch it.
export function useWorkstreamDocList(workstreamId: string | null) {
  return useQuery({
    queryKey: ["docs", "workstream", workstreamId],
    queryFn: () => unwrap(commands.listWorkstreamDocs(workstreamId ?? "")),
    enabled: workstreamId !== null,
  });
}

// Under the ["docs"] prefix so every doc-list invalidation refetches its pages.
export function useDocPages(query: ThoughtQuery) {
  return useInfiniteQuery({
    queryKey: ["docs", "pages", query],
    queryFn: ({ pageParam }) => unwrap(commands.queryDocs({ ...query, offset: pageParam })),
    initialPageParam: 0,
    getNextPageParam: (last, pages) => {
      const loaded = pages.reduce((n, p) => n + p.items.length, 0);
      return loaded < last.total && last.items.length > 0 ? loaded : undefined;
    },
    placeholderData: keepPreviousData,
  });
}

export function useDocCounts(project: string | undefined, filters: ThoughtQuery | null = null) {
  return useQuery({
    queryKey: ["doc-counts", project ?? null, filters],
    queryFn: () => unwrap(commands.countDocs(project ?? null, filters ?? {})),
  });
}

function invalidateDocLists() {
  void queryClient.invalidateQueries({ queryKey: ["docs"] });
  void queryClient.invalidateQueries({ queryKey: ["doc-counts"] });
}

// Patched, not refetched: a read toggle never changes the bytes worth re-reading.
function setCachedUnread(id: string, unread: boolean) {
  queryClient.setQueryData<DocContent>(["doc", id], (d) => d && { ...d, meta: { ...d.meta, unread } });
}

export function useMarkDocRead() {
  return useMutation({
    mutationFn: (id: string) => unwrap(commands.markDocRead(id)),
    onSuccess: (_, id) => setCachedUnread(id, false),
    onSettled: invalidateDocLists,
  });
}

export function useMarkDocUnread() {
  return useMutation({
    mutationFn: (id: string) => unwrap(commands.markDocUnread(id)),
    onSuccess: (_, id) => setCachedUnread(id, true),
    onSettled: invalidateDocLists,
  });
}

export function useDoc(id: string, keepPrevious = false) {
  return useQuery({
    queryKey: ["doc", id],
    queryFn: () => unwrap(commands.getDoc(id)),
    enabled: id.length > 0,
    placeholderData: keepPrevious && id ? keepPreviousData : undefined,
  });
}

export function fetchDocFresh(id: string) {
  return queryClient.fetchQuery({ queryKey: ["doc", id], queryFn: () => unwrap(commands.getDoc(id)), staleTime: 0 });
}

export function useSaveDocParts(id: string) {
  return useMutation({
    // mtimeMs is `number | null` in the bindings; null (unreadable metadata) is sent as 0.
    mutationFn: (v: { body: string; frontmatter: FrontmatterEditDto; baseMtimeMs: number }) =>
      unwrap(commands.saveDocParts(id, v.body, v.frontmatter, v.baseMtimeMs)),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["docs"] });
      void queryClient.invalidateQueries({ queryKey: ["related", id] });
    },
  });
}

export function useRelatedDocs(id: string, values: string[]) {
  return useQuery({
    queryKey: ["related", id, values],
    queryFn: () => unwrap(commands.resolveRelatedDocs(id, values)),
    enabled: id.length > 0 && values.length > 0,
    placeholderData: keepPreviousData,
  });
}

export function prefetchDoc(id: string) {
  void queryClient.prefetchQuery({ queryKey: ["doc", id], queryFn: () => unwrap(commands.getDoc(id)) });
}

// Used by the related picker to resolve pick candidates against the same
// ["related", id, values] cache useRelatedDocs reads, without mounting a query.
export function resolveRelatedOnce(id: string, values: string[]) {
  return queryClient.fetchQuery({
    queryKey: ["related", id, values],
    queryFn: () => unwrap(commands.resolveRelatedDocs(id, values)),
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
    placeholderData: id ? keepPreviousData : undefined,
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
      invalidateLeads();
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

export function useAccountStatus() {
  return useQuery({
    queryKey: ["account"],
    queryFn: () => unwrap(commands.accountStatus()),
    staleTime: 60_000,
  });
}

export function useAccountSignIn() {
  return useMutation({
    mutationKey: ["account-sign-in"],
    mutationFn: () => unwrap(commands.accountSignIn()),
    onSuccess: (status) => queryClient.setQueryData(["account"], status),
  });
}

export function useAccountCancelSignIn() {
  return useMutation({ mutationFn: () => commands.accountCancelSignIn() });
}

export function useAccountSignOut() {
  return useMutation({
    mutationFn: () => unwrap(commands.accountSignOut()),
    onSuccess: (status) => queryClient.setQueryData(["account"], status),
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
  invalidateLeads();
}

export function invalidateLeads() {
  void queryClient.invalidateQueries({ queryKey: ["leads"] });
}

export function useLeads(enabled = true) {
  return useQuery({
    queryKey: ["leads"],
    queryFn: () => unwrap(commands.listLeads()),
    enabled,
  });
}

export function useMessageLead() {
  return useMutation({
    mutationFn: ({ projectPath, text }: { projectPath: string; text: string }) =>
      unwrap(commands.messageLead(projectPath, text)),
    onSuccess: () => invalidateWorkstreamLists(),
  });
}

export function useSetLeadRemoteControl() {
  return useMutation({
    mutationFn: ({ projectPath, enabled }: { projectPath: string; enabled: boolean }) =>
      unwrap(commands.setLeadRemoteControl(projectPath, enabled)),
    onSettled: () => invalidateLeads(),
  });
}

// The event names only the project, so an archive made elsewhere shows as an id that left its list.
export async function refreshProjectWorkstreams(projectPath: string): Promise<void> {
  const ids = () =>
    new Set(queryClient.getQueryData<WorkstreamView[]>(["workstreams", projectPath])?.map((w) => w.item.id));
  const before = ids();
  invalidateLeads();
  await queryClient.invalidateQueries({ queryKey: ["workstreams"] });
  const after = ids();
  for (const id of before) {
    if (!after.has(id)) for (const cleanup of workstreamArchiveCleanups) cleanup(id);
  }
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

export function useWorkstreamPromptContext(workstreamId: string | null, enabled: boolean) {
  return useQuery({
    queryKey: ["workstream-prompt-context", workstreamId],
    queryFn: () => unwrap(commands.workstreamPromptContext(workstreamId!)),
    enabled: enabled && workstreamId !== null,
    staleTime: 30_000,
    refetchOnWindowFocus: false,
  });
}

export function workstreamSessionsQuery(workstreamId: string) {
  return {
    queryKey: ["workstream-sessions", workstreamId],
    queryFn: () => unwrap(commands.listSessions(workstreamId)),
  };
}

export function useWorkstreamSessions(workstreamId: string, enabled = true) {
  return useQuery({
    ...workstreamSessionsQuery(workstreamId),
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

// Unhooked agents publish no change events, so every mount refetches.
export function usePtyAgents(workstreamId: string | null, enabled = true) {
  return useQuery({
    queryKey: ["pty-agents", workstreamId],
    queryFn: () => unwrap(commands.listPtyAgents(workstreamId)),
    enabled: enabled && workstreamId !== "",
    staleTime: 0,
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
    mutationFn: async ({ id, title, branch }: { id: string; title: string; branch: boolean }) => {
      if (branch) await unwrap(commands.renameWorkstreamBranch(id, title));
      await unwrap(commands.renameWorkstream(id, title));
    },
    onSettled: (_data, _error, { id }) => invalidateWorkstream(id),
  });
}

// A server too old for the command errors, which leaves the dialog without the branch option.
export function useBranchRenamePreview(id: string, title: string, enabled: boolean) {
  return useQuery({
    queryKey: ["branch-rename", id, title],
    queryFn: () => unwrap(commands.previewBranchRename(id, title)),
    enabled: enabled && id.length > 0 && title.length > 0,
    retry: false,
    placeholderData: (prev) => prev,
  });
}

const STAGE_SETTLE_MS = 500;
const unsettledStages = new Map<string, ReturnType<typeof setTimeout>>();

// Cycling modes passes through stages and the stored stage only moves forward, so only the settled pick writes.
export function settleWorkstreamStage(id: string, stage: Stage) {
  clearTimeout(unsettledStages.get(id));
  unsettledStages.set(
    id,
    setTimeout(() => {
      unsettledStages.delete(id);
      void commands.setWorkstreamStage(id, stage).finally(invalidateWorkstreamLists);
    }, STAGE_SETTLE_MS),
  );
}

// No optimistic cancel: the create's list refetch must land, or ProjectPage drops the new selection.
export async function setNewWorkstreamStage(id: string, stage: Stage): Promise<void> {
  await unwrap(commands.setWorkstreamStage(id, stage));
  invalidateWorkstreamLists();
}

export function useArchiveWorkstream() {
  return useMutation({
    mutationFn: async ({
      id,
      skipScripts = false,
      childIds = [],
    }: {
      id: string;
      skipScripts?: boolean;
      childIds?: string[];
    }) => {
      for (const child of childIds) {
        await unwrap(commands.archiveWorkstream(child, skipScripts));
        for (const cleanup of workstreamArchiveCleanups) cleanup(child);
        invalidateWorkstream(child);
      }
      await unwrap(commands.archiveWorkstream(id, skipScripts));
    },
    onSuccess: (_data, { id }) => {
      for (const cleanup of workstreamArchiveCleanups) cleanup(id);
      invalidateWorkstream(id);
    },
  });
}

export function useChildWorkstreams(id: string | null) {
  return useQuery({
    queryKey: ["workstreams", "@children", id],
    queryFn: id === null ? skipToken : () => unwrap(commands.listChildWorkstreams(id)),
    staleTime: 0,
  });
}

const ARCHIVED_KEY = ["workstreams", "@archived"];

export function useArchivedWorkstreams(enabled = true) {
  return useQuery({
    queryKey: ARCHIVED_KEY,
    queryFn: () => unwrap(commands.listArchivedWorkstreams()),
    enabled,
  });
}

/** Measures new archives and re-asks gh about PRs that could still change. */
export function useRefreshArchivedWorkstreams() {
  return useMutation({
    mutationFn: () => unwrap(commands.refreshArchivedWorkstreams()),
    onSuccess: (changed) => {
      if (changed > 0) void queryClient.invalidateQueries({ queryKey: ARCHIVED_KEY });
    },
  });
}

export function useKeepArchivedWorkstream() {
  return useMutation({
    mutationFn: (id: string) => unwrap(commands.keepArchivedWorkstream(id)),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ARCHIVED_KEY }),
  });
}

export function usePurgeArchivedWorkstreams() {
  return useMutation({
    mutationFn: (ids: string[]) => unwrap(commands.purgeArchivedWorkstreams(ids)),
    onSettled: () => invalidateWorkstreamLists(),
  });
}

export function useDiscardProjectChanges(projectPath: string) {
  return useMutation({
    mutationFn: () => unwrap(commands.discardProjectChanges(projectPath)),
  });
}

/** `target` restores somewhere other than the original path. */
export function useRestoreWorkstream() {
  return useMutation({
    mutationFn: ({ id, target = null }: { id: string; target?: string | null }) =>
      unwrap(commands.restoreWorkstream(id, target)),
    onSuccess: (_data, { id }) => invalidateWorkstream(id),
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
      return pollMs && query.state.data?.kind !== "found" ? pollMs : false;
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

export function usePullWorkstreamBranch(id: string) {
  return useMutation({
    mutationFn: () => unwrap(commands.pullWorkstreamBranch(id)),
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

export function useProjectMemory(workstreamId: string, enabled = true) {
  return useQuery({
    queryKey: ["workstream", workstreamId, "memory"],
    queryFn: () => unwrap(commands.listProjectMemory(workstreamId)),
    enabled: enabled && workstreamId.length > 0,
  });
}

export function useSaveProjectMemoryFile(workstreamId: string, path: string) {
  return useMutation({
    mutationFn: ({ name, content, baseMtimeMs }: { name: string; content: string; baseMtimeMs: number }) =>
      unwrap(commands.saveProjectMemoryFile(workstreamId, name, content, baseMtimeMs)),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["doc", path] });
      void queryClient.invalidateQueries({ queryKey: ["workstream", workstreamId, "memory"] });
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

export function useReviewChangedFiles(id: string, enabled = true) {
  return useQuery({
    queryKey: ["workstream", id, "review", "files"],
    queryFn: () => unwrap(commands.reviewChangedFiles(id)),
    enabled: enabled && id.length > 0,
  });
}

// Whole files in memory: let them go soon after the diff pane leaves them.
export function useReviewFileSides(id: string, path: string, enabled = true) {
  return useQuery({
    queryKey: ["workstream", id, "review", "sides", path],
    queryFn: () => unwrap(commands.reviewFileSides(id, path)),
    enabled: enabled && id.length > 0 && path.length > 0,
    gcTime: 60_000,
  });
}

export function invalidateReview(id: string) {
  void queryClient.invalidateQueries({ queryKey: ["workstream", id, "review", "state"] });
}

// Creates the review on first read; refreshes on `reviewChanged` and with the worktree.
export function useReview(id: string, enabled = true) {
  return useQuery({
    queryKey: ["workstream", id, "review", "state"],
    queryFn: () => unwrap(commands.getReview(id)),
    enabled: enabled && id.length > 0,
  });
}

// One `claude -p` call per report: a report never changes once written, so its summary never goes stale.
export function useValidationSummary(reportId: string | null, text: string | null) {
  return useQuery({
    queryKey: ["validation-summary", reportId],
    queryFn: () => unwrap(commands.summarizeValidationReport(text!)),
    enabled: reportId !== null && text !== null,
    staleTime: Infinity,
    gcTime: Infinity,
    retry: false,
  });
}

// The workstream id travels in the variables, never a hook argument, so a settled
// mutation invalidates the review it changed.
export function useSetReviewFileViewed() {
  return useMutation({
    mutationFn: (req: SetReviewFileViewedRequest) => unwrap(commands.setReviewFileViewed(req)),
    onSettled: (_data, _error, req) => invalidateReview(req.workstreamId),
  });
}

// Sequential, with one refetch at the end rather than one per file.
export function useMarkReviewFilesViewed() {
  return useMutation({
    mutationFn: async (req: { workstreamId: string; files: { path: string; contentOid: string }[] }) => {
      for (const f of req.files) {
        await unwrap(
          commands.setReviewFileViewed({
            workstreamId: req.workstreamId,
            path: f.path,
            contentOid: f.contentOid,
            viewed: true,
          }),
        );
      }
    },
    onSettled: (_data, _error, req) => invalidateReview(req.workstreamId),
  });
}

export function useAddReviewComment() {
  return useMutation({
    mutationFn: (req: AddReviewCommentRequest) => unwrap(commands.addReviewComment(req)),
    onSettled: (_data, _error, req) => invalidateReview(req.workstreamId),
  });
}

export function useEditReviewComment() {
  return useMutation({
    mutationFn: (v: { workstreamId: string; commentId: string; body: string }) =>
      unwrap(commands.editReviewComment(v.workstreamId, v.commentId, v.body)),
    onSettled: (_data, _error, v) => invalidateReview(v.workstreamId),
  });
}

export function useDeleteReviewComment() {
  return useMutation({
    mutationFn: (v: { workstreamId: string; commentId: string }) =>
      unwrap(commands.deleteReviewComment(v.workstreamId, v.commentId)),
    onSettled: (_data, _error, v) => invalidateReview(v.workstreamId),
  });
}

export function useResolveReviewComment() {
  return useMutation({
    mutationFn: (v: { workstreamId: string; commentId: string; resolved: boolean }) =>
      unwrap(commands.resolveReviewComment(v.workstreamId, v.commentId, v.resolved)),
    onSettled: (_data, _error, v) => invalidateReview(v.workstreamId),
  });
}

export function useResolveReviewNote() {
  return useMutation({
    mutationFn: (v: { workstreamId: string; noteId: string; resolved: boolean }) =>
      unwrap(commands.resolveReviewNote(v.workstreamId, v.noteId, v.resolved)),
    onSettled: (_data, _error, v) => invalidateReview(v.workstreamId),
  });
}

export function useSubmitReview() {
  return useMutation({
    mutationFn: (req: SubmitReviewRequest) => unwrap(commands.submitReview(req)),
    onSettled: (_data, _error, req) => invalidateReview(req.workstreamId),
  });
}

export function useMarkReviewRoundDelivered() {
  return useMutation({
    mutationFn: (v: { workstreamId: string; round: number }) =>
      unwrap(commands.markReviewRoundDelivered(v.workstreamId, v.round)),
    onSettled: (_data, _error, v) => invalidateReview(v.workstreamId),
  });
}


export function invalidateGoals(goalId?: string) {
  void queryClient.invalidateQueries({ queryKey: ["goals"] });
  void queryClient.invalidateQueries({ queryKey: goalId ? ["goal", goalId] : ["goal"] });
}

function settleGoal(goalId: string | undefined) {
  invalidateGoals(goalId);
  void queryClient.invalidateQueries({ queryKey: ["workstream-sessions"] });
  void queryClient.invalidateQueries({ queryKey: ["active-sessions"] });
}

// Newest first, as `list_goals` returns them; null lists every workstream's.
export function useGoals(workstreamId: string | null, activeOnly = false, enabled = true) {
  return useQuery({
    queryKey: ["goals", workstreamId, activeOnly],
    queryFn: () => unwrap(commands.listGoals(workstreamId, activeOnly)),
    enabled,
  });
}

export function useGoal(goalId: string | null) {
  return useQuery({
    queryKey: ["goal", goalId],
    queryFn: () => unwrap(commands.getGoal(goalId!)),
    enabled: goalId !== null,
  });
}

// The goal id travels in the variables, never a hook argument, so a settled
// mutation invalidates the goal it changed.
export function useStartGoal() {
  return useMutation({
    mutationFn: (request: StartGoalRequest) => unwrap(commands.startGoal(request)),
    onSettled: (goal) => settleGoal(goal?.id),
  });
}

export function usePauseGoal() {
  return useMutation({
    mutationFn: (goalId: string) => unwrap(commands.pauseGoal(goalId)),
    onSettled: (_data, _error, goalId) => settleGoal(goalId),
  });
}

export function useResumeGoal() {
  return useMutation({
    mutationFn: (goalId: string) => unwrap(commands.resumeGoal(goalId)),
    onSettled: (_data, _error, goalId) => settleGoal(goalId),
  });
}

export function useStopGoal() {
  return useMutation({
    mutationFn: (goalId: string) => unwrap(commands.stopGoal(goalId)),
    onSettled: (_data, _error, goalId) => settleGoal(goalId),
  });
}

export function useUpdateGoal() {
  return useMutation({
    mutationFn: (v: { goalId: string; patch: GoalPatch }) =>
      unwrap(commands.updateGoal(v.goalId, v.patch)),
    onSettled: (_data, _error, v) => settleGoal(v.goalId),
  });
}

// A query, not a mutation: StrictMode's remount detaches a mutation fired from a mount effect.
export function useDeriveGoal(request: DeriveGoalRequest | null) {
  return useQuery({
    queryKey: ["goal-derive", request],
    queryFn: () => unwrap(commands.deriveGoal(request as DeriveGoalRequest)),
    enabled: request !== null,
    staleTime: Infinity,
    gcTime: 0,
    retry: false,
  });
}


export function useAiStatus() {
  return useQuery({
    queryKey: ["aiStatus"],
    queryFn: () => unwrap(commands.getAiStatus()),
    staleTime: Infinity,
  });
}

export function useAgentFiles(projectPath: string | null) {
  return useQuery({
    queryKey: ["agent-files", projectPath],
    queryFn: () => unwrap(commands.listAgentFiles(projectPath)),
  });
}

export function useAgentSkills(projectPath: string | null) {
  return useQuery({
    queryKey: ["agent-skills", projectPath],
    queryFn: () => unwrap(commands.agentSkills(projectPath)),
  });
}

export function useSkillFiles(projectPath: string | null) {
  return useQuery({
    queryKey: ["skill-files", projectPath],
    queryFn: () => unwrap(commands.listSkillFiles(projectPath)),
  });
}

export function useSkillFileText(projectPath: string | null, path: string) {
  return useQuery({
    queryKey: ["skill-file", projectPath, path],
    queryFn: () => unwrap(commands.readSkillFile(projectPath, path)),
  });
}

export function useSkillAssetText(projectPath: string | null, skillPath: string, relPath: string) {
  return useQuery({
    queryKey: ["skill-asset", projectPath, skillPath, relPath],
    queryFn: () => unwrap(commands.readSkillAsset(projectPath, skillPath, relPath)),
  });
}

export function invalidateSkills(): void {
  void queryClient.invalidateQueries({ queryKey: ["skill-files"] });
  void queryClient.invalidateQueries({ queryKey: ["skill-file"] });
  void queryClient.invalidateQueries({ queryKey: ["skill-asset"] });
  void queryClient.invalidateQueries({ queryKey: ["agent-skills"] });
  void queryClient.invalidateQueries({ queryKey: ["skill-locks"] });
  void queryClient.invalidateQueries({ queryKey: ["skillfish-candidates"] });
  // Disabled, so invalidating would leave stale "Update" badges; a reset drops them until re-checked.
  void queryClient.resetQueries({ queryKey: ["skill-updates"] });
}

export function usePreviewSkillInstall() {
  return useMutation({
    mutationFn: (v: { projectPath: string | null; source: SkillSourceInput }) =>
      unwrap(commands.previewSkillInstall(v.projectPath, v.source)),
  });
}

export function useInstallSkill() {
  return useMutation({
    mutationFn: (v: { projectPath: string | null; token: string; scope: SkillScope; targets: SkillTarget[] }) =>
      unwrap(commands.installSkill(v.projectPath, v.token, v.scope, v.targets)),
    onSuccess: invalidateSkills,
  });
}

export function useRemoveSkill() {
  return useMutation({
    mutationFn: (v: { projectPath: string | null; scope: SkillScope; name: string }) =>
      unwrap(commands.removeSkill(v.projectPath, v.scope, v.name)),
    onSuccess: invalidateSkills,
  });
}

export function useSkillLocks(projectPath: string | null) {
  return useQuery({
    queryKey: ["skill-locks", projectPath],
    queryFn: () => unwrap(commands.listSkillLocks(projectPath)),
  });
}

// Network: runs only when `refetch()` is called.
export function useSkillUpdates(projectPath: string | null) {
  return useQuery({
    queryKey: ["skill-updates", projectPath],
    queryFn: () => unwrap(commands.checkSkillUpdates(projectPath)),
    enabled: false,
    staleTime: Infinity,
    retry: false,
  });
}

export function usePreviewSkillUpdate() {
  return useMutation({
    mutationFn: (v: { projectPath: string | null; scope: SkillScope; name: string }) =>
      unwrap(commands.previewSkillUpdate(v.projectPath, v.scope, v.name)),
  });
}

export function useRestoreSkills() {
  return useMutation({
    mutationFn: (v: { projectPath: string | null; scope: SkillScope }) =>
      unwrap(commands.restoreSkills(v.projectPath, v.scope)),
    onSuccess: invalidateSkills,
  });
}

export function useSkillfishCandidates(projectPath: string | null, enabled: boolean) {
  return useQuery({
    queryKey: ["skillfish-candidates", projectPath],
    queryFn: () => unwrap(commands.listSkillfishCandidates(projectPath)),
    enabled,
  });
}

export function useSkillSourceSearch(query: string) {
  return useQuery({
    queryKey: ["skill-source-search", query],
    queryFn: () => unwrap(commands.searchSkillSource(query)),
    enabled: query.trim().length >= 2,
    staleTime: 5 * 60_000,
    placeholderData: keepPreviousData,
    retry: false,
  });
}

export function useSkillLeaderboard(view: SkillsShView, enabled: boolean) {
  return useInfiniteQuery({
    queryKey: ["skill-leaderboard", view],
    queryFn: ({ pageParam }) => unwrap(commands.skillLeaderboard(view, pageParam)),
    initialPageParam: 0,
    getNextPageParam: (last, _pages, page) => (last.hasMore ? page + 1 : undefined),
    enabled,
    staleTime: 10 * 60_000,
    retry: false,
  });
}

// An older server rejects the command; callers hide counts on error.
export function useAgentHarnessCounts() {
  return useQuery({
    queryKey: ["agent-harness-counts"],
    queryFn: () => unwrap(commands.agentHarnessCounts()),
    staleTime: Infinity,
    retry: false,
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
  void queryClient.invalidateQueries({ queryKey: ["skill-files"] });
  void queryClient.invalidateQueries({ queryKey: ["skill-file"] });
  void queryClient.invalidateQueries({ queryKey: ["agent-harness-counts"] });
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
