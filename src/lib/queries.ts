import { keepPreviousData, useMutation, useQuery } from "@tanstack/react-query";

import {
  commands,
  type ActiveSessionView,
  type AppSettings,
  type BinaryKind,
  type Bootstrap,
  type CliInstallMethodId,
  type CreateDocInput,
  type FmPatchDto,
  type MergeMethod,
  type ScriptKind,
  type ScriptSettings,
  type Stage,
  type WorktreeMode,
  type ThoughtQuery,
  type ThoughtsSource,
  type WorkstreamView,
} from "./bindings";
import { unwrap } from "./ipc";
import { queryClient } from "./queryClient";

/** Feature-side cleanups to run after a workstream archives (e.g. the session
 *  feature releases its in-memory transcripts). Registered from main.tsx so
 *  this module never imports feature stores (the import direction is
 *  routes → features → lib; see docs/adr/0003-frontend-layering.md). */
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

/** The default shell's name (zsh/bash/pwsh/…), to label the dock's ad-hoc
 *  terminal tabs by the shell they spawn. Static per session. */
export function useShellName() {
  return useQuery({
    queryKey: ["shell-name"],
    queryFn: () => unwrap(commands.defaultShellName()),
    staleTime: Infinity,
  });
}

/** Installed editors/terminals for the "Open in…" menu. Apps rarely come and
 *  go mid-session, so this is fetched once and kept until the app reloads. */
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
      // A sync can rewrite the hyprlayer config (e.g. orphan-mapping cleanup),
      // which the sidebar renders from; don't wait for the watcher poll.
      void queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
    },
  });
}

/** Local-only git status of the thoughts repo (branch/dirty/ahead/behind).
 *  Computed by libgit2 with no network I/O, so the Library refreshes it
 *  eagerly while open. `retry: false` because failures are structural (non-git
 *  backend, missing repo), not transient. */
export function useThoughtsGitStatus(path: string | undefined) {
  return useQuery({
    queryKey: ["thoughts-git-status", path],
    queryFn: () => unwrap(commands.getThoughtsGitStatus(path ?? "")),
    enabled: !!path,
    retry: false,
    refetchInterval: 30_000,
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

/** Best-effort "View on GitHub" URL for a doc, or null when no GitHub-shaped
 *  origin resolves (the backend degrades to None rather than erroring). The
 *  doc's repo/remote rarely changes mid-session, so cache generously. */
export function useDocWebUrl(id: string) {
  return useQuery({
    queryKey: ["doc-web-url", id],
    queryFn: () => unwrap(commands.docWebUrl(id)),
    enabled: id.length > 0,
    staleTime: 5 * 60_000,
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

/** hyprlayer CLI install options for this OS, flagged by manager availability.
 *  Drives the onboarding install step. */
export function useCliInstallMethods() {
  return useQuery({
    queryKey: ["cli-install-methods"],
    queryFn: () => unwrap(commands.detectCliInstallMethods()),
  });
}

/** Install the hyprlayer CLI via a package manager (background, no terminal).
 *  Re-detects on success so the install gate flips to "installed". */
export function useInstallHyprlayerCli() {
  return useMutation({
    mutationFn: (method: CliInstallMethodId) => unwrap(commands.installHyprlayerCli(method)),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
      void queryClient.invalidateQueries({ queryKey: ["health"] });
    },
  });
}

/** Install OpenCode in the background (no terminal). The backend re-detects on
 *  success; refresh the bootstrap cache so the status badge flips to Installed. */
export function useInstallOpencode() {
  return useMutation({
    mutationFn: () => unwrap(commands.installOpencode()),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
      void queryClient.invalidateQueries({ queryKey: ["health"] });
    },
  });
}

/** Persist app settings (whole-object replace) and refresh the bootstrap cache. */
export function useSetAppSettings() {
  return useMutation({
    // Whole-object replace: without ordering, a slow earlier write (e.g. a
    // debounced prompt-body save) can land AFTER a fast later one (a pin
    // toggle) and revert its delta on disk. The shared scope makes React Query
    // queue same-scope mutations and run them serially in dispatch order, so
    // the last-dispatched payload is also the final on-disk state.
    scope: { id: "app-settings" },
    mutationFn: (settings: AppSettings) => unwrap(commands.setAppSettings(settings)),
    onSuccess: (next) => {
      // Optimistically fold the saved settings into the cached bootstrap so
      // the toggles don't flicker back while the refetch is in flight.
      queryClient.setQueryData<Bootstrap>(["bootstrap"], (prev) =>
        prev ? { ...prev, settings: next } : prev,
      );
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ["bootstrap"] });
    },
  });
}

/** The authenticated GitHub login, for the "username/" branch-prefix option. */
export function useGithubUsername() {
  return useQuery({
    queryKey: ["github-username"],
    queryFn: () => unwrap(commands.githubUsername()),
    staleTime: 10 * 60_000,
  });
}

/** The shell username ($USER), to prefill the onboarding name field. Matches
 *  the CLI's default. Never changes during a session, so cache indefinitely.
 *  Returns a plain string (the command is infallible). */
export function useDefaultUsername() {
  return useQuery({
    queryKey: ["default-username"],
    queryFn: () => commands.defaultUsername(),
    staleTime: Infinity,
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
    // Each fetch fans a git2 status sweep across every active workstream, so
    // don't re-run it on every app-switch: mutations and session events already
    // invalidate it (see invalidateWorkstream / invalidateWorktreeGit), and a
    // 30s window still picks up out-of-band changes on the next focus.
    staleTime: 30_000,
  });
}

/** All agents currently live in the registry across every project. Used by the
 *  search palette's Agents section. Invalidated by SessionLifecycle events. */
export function useActiveSessions(enabled = true) {
  return useQuery<ActiveSessionView[]>({
    queryKey: ["active-sessions"],
    queryFn: () => unwrap(commands.listActiveSessions()),
    enabled,
  });
}

/** Base-branch choices for the new-workstream dialog (local + remote + the
 * detected default). Fetched lazily — only while the dialog is open. */
export function useWorkstreamBases(projectPath: string, enabled = true) {
  return useQuery({
    queryKey: ["workstream-bases", projectPath],
    queryFn: () => unwrap(commands.listWorkstreamBases(projectPath)),
    enabled,
    staleTime: 30_000,
  });
}

/** How a new workstream is backed. `isolated` creates a fresh branch + worktree;
 * `existing` checks a chosen local branch out into a worktree; `inplace` runs in
 * the project's own checkout with no worktree. */

export function useCreateWorkstream(projectPath: string) {
  return useMutation({
    mutationFn: ({
      title,
      base,
      mode,
      branch,
    }: {
      title: string;
      base: string | null;
      mode: WorktreeMode;
      branch: string | null;
    }) => unwrap(commands.createWorkstream(projectPath, title, base, mode, branch)),
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
    // `skipScripts` runs the archive without the project's Archive script — used
    // by the dialog's "Skip" path after the script failed once.
    mutationFn: ({ id, skipScripts = false }: { id: string; skipScripts?: boolean }) =>
      unwrap(commands.archiveWorkstream(id, skipScripts)),
    onSuccess: (_data, { id }) => {
      // Archiving kills the workstream's live sessions on the backend; the
      // session feature's registered cleanup releases its in-memory transcripts.
      for (const cleanup of workstreamArchiveCleanups) cleanup(id);
      invalidateWorkstream(id);
    },
  });
}

/** Archived workstreams across all projects, newest-first — feeds the
 *  Archive history view. Separate query key from the active lists so
 *  archive/restore mutations invalidate both halves cleanly. */
export function useArchivedWorkstreams(enabled = true) {
  return useQuery({
    queryKey: ["workstreams", "@archived"],
    queryFn: () => unwrap(commands.listArchivedWorkstreams()),
    enabled,
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

// ── Pull request (derived live from gh) ─────────────────────────────────────

/** The branch's PR (or null), derived live from `gh`. Fetches once on mount;
 *  liveness is push-based after that — the backend PR watcher diffs each poll
 *  and emits `prStatusChanged`, which invalidates this query (see
 *  `src/lib/events.ts`). So the bar advances on its own (checks settling, a
 *  merge landing elsewhere flipping Merge → Archive) without a UI timer. */
export function useWorkstreamPr(id: string, enabled = true) {
  return useQuery({
    queryKey: ["workstream", id, "pr"],
    queryFn: () => unwrap(commands.workstreamPrStatus(id)),
    enabled: enabled && id.length > 0,
    // Reuse cached status across rapid workstream switches instead of re-running
    // the `gh` subprocess each time; the backend watcher pushes updates between.
    staleTime: 8000,
  });
}

export function useMergePr(id: string) {
  return useMutation({
    mutationFn: (method: MergeMethod) => unwrap(commands.mergeWorkstreamPr(id, method)),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["workstream", id, "pr"] });
      invalidateWorkstream(id);
    },
  });
}

/** Open PRs in a workstream's repo for the "Attach PR" picker, filtered live by
 *  `query` (GitHub search syntax). Enabled only while the picker is open. */
export function useWorkstreamOpenPrs(id: string, query: string, enabled = true) {
  return useQuery({
    queryKey: ["workstream", id, "open-prs", query],
    queryFn: () => unwrap(commands.listWorkstreamOpenPrs(id, query)),
    enabled: enabled && id.length > 0,
    // Each `gh pr list` is a subprocess; keep results briefly so reopening the
    // picker or backspacing to a prior query doesn't re-run it.
    staleTime: 15_000,
  });
}

/** Live preview of the PR a user typed into the attach picker (by number), or
 *  null when no such PR exists — confirms the number before attaching. */
export function useWorkstreamPrPreview(id: string, prNumber: number | null, enabled = true) {
  return useQuery({
    queryKey: ["workstream", id, "pr-preview", prNumber],
    queryFn: () => unwrap(commands.previewWorkstreamPr(id, prNumber as number)),
    enabled: enabled && id.length > 0 && prNumber !== null,
    staleTime: 15_000,
  });
}

/** Attach a PR (by number) to a workstream, then refresh the PR bar + sidebar.
 *  Errors (e.g. unknown number) surface to the dialog via the mutation state. */
export function useAttachPr(id: string) {
  return useMutation({
    mutationFn: (prNumber: number) => unwrap(commands.attachWorkstreamPr(id, prNumber)),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["workstream", id, "pr"] });
      invalidateWorkstream(id);
    },
  });
}

/** Detach a workstream's manually attached PR, reverting to branch detection.
 *  Takes the id as the mutate variable, not a hook arg: this fires from a context
 *  menu that closes on click, so a closed-over id churns to "" (the menu's
 *  fallback) before the async detach resolves — and `onSuccess` would then
 *  invalidate `["workstream", "", "pr"]`, leaving the PR bar stale until the next
 *  window-focus refetch. Reading `id` from the mutation variable pins it. */
export function useDetachPr() {
  return useMutation({
    mutationFn: (id: string) => unwrap(commands.detachWorkstreamPr(id)),
    onSuccess: (_data, id) => {
      void queryClient.invalidateQueries({ queryKey: ["workstream", id, "pr"] });
      invalidateWorkstream(id);
    },
  });
}

/** Re-target a workstream's base (merge target) branch. Re-points an open PR via
 *  `gh pr edit --base` (backend), then refreshes ahead/behind, the PR bar, and the
 *  sidebar's stored base. Errors (e.g. base not on the remote) surface to the UI. */
export function useSetWorkstreamBase(id: string) {
  return useMutation({
    mutationFn: (base: string) => unwrap(commands.setWorkstreamBaseBranch(id, base)),
    onSuccess: () => {
      invalidateWorktreeGit(id); // ahead/behind recompute against the new base
      void queryClient.invalidateQueries({ queryKey: ["workstream", id, "pr"] });
      invalidateWorkstream(id); // sidebar list shows the stored base
    },
  });
}

/** Pull the target branch's new commits into the worktree (fetch + merge). The
 *  resolved `PullOutcome` (data) tells the caller whether it merged cleanly, was
 *  already up to date, or stopped on conflicts to resolve by hand. Refreshes
 *  ahead/behind + the Changes panel (conflicted files surface there). */
export function useUpdateFromBase(id: string) {
  return useMutation({
    mutationFn: () => unwrap(commands.updateWorkstreamFromBase(id)),
    onSuccess: () => {
      invalidateWorktreeGit(id); // ahead/behind drops; merged/conflicted files refresh
      invalidateWorkstream(id);
    },
  });
}

/** Fetch the workstream's remote (`git fetch --prune origin`), then refresh the
 *  remote-derived state: ahead/behind (recomputed against the updated refs) and
 *  the PR view. `onSettled` so an offline/failed fetch still refreshes local
 *  state rather than leaving the panel stale. */
export function useFetchWorkstreamRemote(id: string) {
  return useMutation({
    mutationFn: () => unwrap(commands.fetchWorkstreamRemote(id)),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ["workstream", id, "pr"] });
      invalidateWorktreeGit(id); // also invalidates the workstreams list (ahead/behind)
    },
  });
}

// ── Source-control panel ──────────────────────────────────────────────────

/** Invalidate the things a staging action changes: the split status and the
 *  workstream's own view (its dirty/line counts in the sidebar tree). Exported
 *  so the session store can refresh the Changes panel after an agent turn edits
 *  the worktree. */
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

// ── Onboarding ───────────────────────────────────────────────────────────────

/** AI tool install state — gates the AI step in the onboarding wizard.
 *  Cached per session (staleTime: Infinity) since the install state rarely
 *  changes mid-session; `.refetch()` is called explicitly after `ai configure`
 *  exits 0 in the embedded terminal. */
export function useAiStatus() {
  return useQuery({
    queryKey: ["aiStatus"],
    queryFn: () => unwrap(commands.getAiStatus()),
    staleTime: Infinity,
  });
}

/** Probe `~/thoughts` so the wizard can default to "open from disk" (and prefill
 *  the path) when an existing repo is already there. Re-checked on each mount —
 *  the user may have just created the dir in another step. */
export function useProbeDefaultThoughts() {
  return useQuery({
    queryKey: ["probe-default-thoughts"],
    queryFn: () => commands.probeDefaultThoughts(),
  });
}

/** Set up the user's thoughts repo (clone/disk/create) and write config.json.
 *  Invalidates the full bootstrap/projects/docs/workstreams set on success so
 *  the rest of the app reflects the new ready state immediately. */
// ── Repo worktree file list ───────────────────────────────────────────────────

/** Files/globs configured to be copied from a repo into each new worktree. */
export function useWorktreeFiles(path: string) {
  return useQuery({
    queryKey: ["worktreeFiles", path],
    queryFn: () => unwrap(commands.getWorktreeFiles(path)),
  });
}

/** Persist the full worktree-files list for a project (whole-list replace). */
export function useSetWorktreeFiles(path: string) {
  return useMutation({
    mutationFn: (patterns: string[]) => unwrap(commands.setWorktreeFiles(path, patterns)),
    onSuccess: (next) => {
      queryClient.setQueryData(["worktreeFiles", path], next);
      // The copy list changed — re-validate so the project's warning badge
      // reflects the new patterns immediately.
      void queryClient.invalidateQueries({ queryKey: ["worktree-files-check", path] });
    },
  });
}

/** Dry-run check of a project's worktree-copy patterns against its checkout:
 *  which configured files/globs resolve and which are missing/errored. Drives
 *  the sidebar's per-project "copy files not found" warning. Inherits the global
 *  staleTime (5s) + refetchOnWindowFocus, so it re-checks on app start, when the
 *  copy list changes (invalidated by [`useSetWorktreeFiles`]), AND when the
 *  window regains focus — so fixing a file on disk and returning to the app
 *  clears the warning without a restart. Focus is driven by Tauri's window
 *  signal (see [`setupQueryFocus`]), since the WKWebview doesn't emit the
 *  browser focus event on app switches. */
export function useWorktreeFilesCheck(path: string) {
  return useQuery({
    queryKey: ["worktree-files-check", path],
    queryFn: () => unwrap(commands.checkWorktreeFiles(path)),
  });
}

// ── Project lifecycle scripts (Setup / Run / Archive) ──────────────────────────

/** The project's lifecycle scripts (`[scripts]` in `.hyprlayer/settings.toml`). */
export function useScripts(path: string) {
  return useQuery({
    queryKey: ["scripts", path],
    queryFn: () => unwrap(commands.getScripts(path)),
  });
}

/** Persist the project's lifecycle scripts (whole-object replace). The Settings
 *  card debounces calls into this for autosave. */
export function useSetScripts(path: string) {
  return useMutation({
    mutationFn: (scripts: ScriptSettings) => unwrap(commands.setScripts(path, scripts)),
    onSuccess: (_data, scripts) => {
      queryClient.setQueryData(["scripts", path], scripts);
    },
  });
}

/** A workstream's live script runtime: the running Run process (terminal id +
 *  detected port) and each script's last-run status. Hydrates the dock/rail on
 *  mount; kept fresh by the `scriptRunChanged` event (see events.ts). */
export function useScriptRuntime(workstreamId: string) {
  return useQuery({
    queryKey: ["script-runtime", workstreamId],
    queryFn: () => unwrap(commands.getScriptRuntime(workstreamId)),
    enabled: !!workstreamId,
  });
}

/** Invalidate a workstream's script runtime (called from the scriptRunChanged
 *  event bridge). */
export function invalidateScriptRuntime(workstreamId: string) {
  void queryClient.invalidateQueries({ queryKey: ["script-runtime", workstreamId] });
}

/** Captured output of a script's last run this session, or null. Drives the
 *  dock's read-only Setup view. */
export function useScriptOutput(workstreamId: string, kind: ScriptKind, enabled = true) {
  return useQuery({
    queryKey: ["script-output", workstreamId, kind],
    queryFn: () => unwrap(commands.getScriptOutput(workstreamId, kind)),
    enabled: enabled && !!workstreamId,
  });
}

/** Invalidate a workstream/kind's captured output (from the event bridge). */
export function invalidateScriptOutput(workstreamId: string, kind: ScriptKind) {
  void queryClient.invalidateQueries({ queryKey: ["script-output", workstreamId, kind] });
}

// ── Harnesses (Claude Code + OpenCode) ─────────────────────────────────────

/** The models.dev provider/model catalog backing the OpenCode pickers. Cached
 *  hard (the backend already does disk caching + a bundled fallback); the
 *  "Refresh" affordance forces a network refetch via [`useRefreshOpencodeCatalog`]. */
export function useOpencodeCatalog(enabled = true) {
  return useQuery({
    queryKey: ["opencode-catalog"],
    queryFn: () => unwrap(commands.opencodeCatalog(false)),
    enabled,
    staleTime: 60 * 60_000,
  });
}

/** Force a network refresh of the models.dev catalog (bypasses the disk cache). */
export function useRefreshOpencodeCatalog() {
  return useMutation({
    mutationFn: () => unwrap(commands.opencodeCatalog(true)),
    onSuccess: (next) => queryClient.setQueryData(["opencode-catalog"], next),
  });
}

/** Providers with a credential in OpenCode's auth.json (never the raw keys). */
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

/** Set the OpenCode default model (persists to app settings + mirrors to
 *  opencode.json), then refresh the bootstrap so the composer picks it up. */
export function useSetOpencodeDefaultModel() {
  return useMutation({
    mutationFn: (model: string | null) => unwrap(commands.opencodeSetDefaultModel(model)),
    onSettled: () => queryClient.invalidateQueries({ queryKey: ["bootstrap"] }),
  });
}

/** Reachability + installed models for the configured Ollama endpoint (local or
 *  remote). Keyed by `endpoint` so changing the server (here or elsewhere)
 *  refetches against the new host instead of showing the old one's cache. The
 *  probe is cheap but can change (server up/down, models pulled), so it's lightly
 *  cached and the Ollama tab exposes a Refresh via `refetch`. */
export function useOllamaStatus(endpoint?: string | null, enabled = true) {
  return useQuery({
    queryKey: ["ollama-status", endpoint ?? null],
    queryFn: () => unwrap(commands.ollamaStatus()),
    enabled,
    staleTime: 15_000,
  });
}

/** Set the Ollama base URL (local or remote). The command returns a fresh status
 *  probe of the new endpoint, which we seed into that endpoint's cache key;
 *  settings changed, so refresh the bootstrap too. */
export function useSetOllamaEndpoint() {
  return useMutation({
    mutationFn: (endpoint: string | null) => unwrap(commands.ollamaSetEndpoint(endpoint)),
    onSuccess: (next, endpoint) =>
      queryClient.setQueryData(["ollama-status", endpoint ?? null], next),
    onSettled: () => queryClient.invalidateQueries({ queryKey: ["bootstrap"] }),
  });
}

/** Set which installed Ollama models are exposed in the composer. Mirrors them
 *  into OpenCode's provider block backend-side; refresh bootstrap so the composer
 *  picks up the new list. */
export function useSetOllamaModels() {
  return useMutation({
    mutationFn: (models: string[]) => unwrap(commands.ollamaSetSelectedModels(models)),
    onSettled: () => queryClient.invalidateQueries({ queryKey: ["bootstrap"] }),
  });
}

/** Set the default Ollama model (an `"ollama/<id>"` id), then refresh bootstrap. */
export function useSetOllamaDefaultModel() {
  return useMutation({
    mutationFn: (model: string | null) => unwrap(commands.ollamaSetDefaultModel(model)),
    onSettled: () => queryClient.invalidateQueries({ queryKey: ["bootstrap"] }),
  });
}

/** Current Claude auth/account status (for the Claude Code harness tab). The
 *  command runs `claude auth status --json`, so refetch is explicit (a Refresh
 *  button) rather than on a timer. */
export function useClaudeAuthStatus(enabled = true) {
  return useQuery({
    queryKey: ["claude-auth-status"],
    queryFn: () => unwrap(commands.claudeAuthStatus()),
    enabled,
    staleTime: 5 * 60_000,
  });
}

export function useInitThoughtsRepo() {
  return useMutation({
    mutationFn: (args: { user: string; source: ThoughtsSource }) =>
      unwrap(commands.initThoughtsRepo(args.user, args.source)),
    onSuccess: (data) => {
      // Optimistically hydrate the bootstrap cache so the wizard's success
      // state is instant, then trigger a full invalidation for correctness.
      queryClient.setQueryData<Bootstrap>(["bootstrap"], data);
      invalidateProjects();
    },
  });
}
