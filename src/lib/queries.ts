import { useEffect, useRef } from "react";
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

/** Whether the app holds macOS Full Disk Access. The grant only takes effect on
 *  relaunch, so it cannot change mid-session, fetched once. */
export function useFullDiskAccess() {
  return useQuery({
    queryKey: ["full-disk-access"],
    queryFn: () => commands.fullDiskAccessStatus(),
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

/** Minimum `text` length before a doc query is sent to `query_docs`. Below
 *  this, a text query is a full-corpus disk scan (the index reads a capped
 *  body per candidate from disk), amplified by `refetchOnWindowFocus`, a
 *  single keystroke shouldn't trigger it. Three features gate on this
 *  (command palette, library sidebar, workstream docs panel), so it lives
 *  here per CLAUDE.md's "used by two or more features" rule. */
export const MIN_DOC_TEXT_QUERY = 3;

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

/** Classify a browsed disk path for the Add-project dialog's offer (import,
 *  init, or refuse). Never cached (staleTime 0), the answer can change the
 *  moment the user runs `git init` on disk. */
export function useRepoPathInfo(path: string, enabled = true) {
  return useQuery({
    queryKey: ["repo-path-info", path],
    queryFn: () => unwrap(commands.inspectRepoPath(path)),
    enabled: enabled && path.trim().length > 0,
    staleTime: 0,
  });
}

/** Import a local on-disk folder as a project (registered in place), handling
 *  any git setup the path needs (init, birth a commit, adopt, or refuse).
 *  `thoughtsDir`/`profile` pick the thoughts folder + profile the repo maps
 *  into (null = named after the repo / the default backend). */
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

/** Register a plain local directory (no git, no thoughts, no CLI) as a project. */
export function useImportLocalFolder() {
  return useMutation({
    mutationFn: (args: { path: string; directoryName: string | null }) =>
      unwrap(commands.importLocalFolder(args.path, args.directoryName)),
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

/** The thoughts destinations for the Add-project dialog's Thoughts step: the
 *  default backend + named profiles, each with its existing repo folders. */
export function useThoughtsProfiles(enabled = true) {
  return useQuery({
    queryKey: ["thoughts-profiles"],
    queryFn: () => unwrap(commands.listThoughtsProfiles()),
    enabled,
    staleTime: 30_000,
  });
}

/** Create a named thoughts profile (git backend, standard dirs), the
 *  Add-project dialog's inline "New profile…" form. */
export function useCreateThoughtsProfile() {
  return useMutation({
    mutationFn: (args: { name: string; thoughtsRepo: string | null }) =>
      unwrap(commands.createThoughtsProfile(args.name, args.thoughtsRepo)),
    // Seed the cache so the new profile is selectable before the refetch lands
    // (the dialog selects it by name the moment the mutation succeeds).
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

/** Persist a drag-to-reorder of the project sidebar. `paths` is the full project
 *  list in its new top-to-bottom order. Optimistic: the reordered list stays on
 *  screen (the drag already moved it locally), reverting only if the write fails. */
export function useReorderProjects() {
  return useMutation({
    // Serialize concurrent reorders so a slow earlier write can't land after a
    // fast later one and revert it (same rationale as useSetAppSettings).
    scope: { id: "reorder-projects" },
    mutationFn: (paths: string[]) => unwrap(commands.reorderProjects(paths)),
    onMutate: async (paths) => {
      // Cancel in-flight bootstrap refetches so they can't clobber our order.
      await queryClient.cancelQueries({ queryKey: ["bootstrap"] });
      const previous = queryClient.getQueryData<Bootstrap>(["bootstrap"]);
      queryClient.setQueryData<Bootstrap>(["bootstrap"], (prev) => {
        if (!prev) return prev;
        const byPath = new Map(prev.projects.map((p) => [p.path, p]));
        const ordered = paths
          .map((path) => byPath.get(path))
          .filter((p): p is Project => p !== undefined);
        // Never drop a row: keep any project not named in `paths` (e.g. one added
        // out-of-band mid-drag) at the end.
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
    onMutate: async (settings) => {
      // Fold the payload in BEFORE the round-trip, not just on success: a
      // caller builds its next payload by patching the cached settings, so a
      // second click inside one round-trip would otherwise read the pre-first
      // state and, because the scope runs these serially, land last and
      // silently revert the first. Rapid folder collapses hit this directly.
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
      // The saved settings as the backend normalized them, which may differ
      // from what we optimistically wrote.
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
//
// The gh-backed PR queries deliberately sit OUTSIDE that prefix, under
// ["workstream-pr", id], see `invalidateWorkstreamPr`.

/** Refresh every workstream list: the project's board AND the sidebar's
 *  cross-project tree. Only one of them performs a given backend-side
 *  reconciliation, so the other needs telling. */
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

/** Active workstreams across all projects, the sidebar tree. */
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

/** A workstream's sessions. The session row is the AUTHORITATIVE record of which
 *  harness a chat runs on (the backend pins it at launch and resumes by it), so
 *  this is what the composer reads for an already-started tab rather than
 *  re-deriving the harness from the current default. */
export function useWorkstreamSessions(workstreamId: string, enabled = true) {
  return useQuery({
    queryKey: ["workstream-sessions", workstreamId],
    queryFn: () => unwrap(commands.listSessions(workstreamId)),
    enabled: enabled && Boolean(workstreamId),
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
 * detected default). Fetched lazily, only while the dialog is open. Never
 * cached (staleTime 0) so a branch created on disk while the app is open
 * shows up the next time the picker opens. */
export function useWorkstreamBases(projectPath: string, enabled = true) {
  return useQuery({
    queryKey: ["workstream-bases", projectPath],
    queryFn: () => unwrap(commands.listWorkstreamBases(projectPath)),
    enabled,
    staleTime: 0,
  });
}

/** `git fetch --prune origin` in the project's checkout, so the branch picker sees
 * branches pushed since the app last fetched. Fired when the new-workstream dialog
 * opens; the refreshed branch list arrives via the invalidation below. Failure is
 * the caller's to ignore, an offline fetch just leaves the on-disk refs in place. */
export function useFetchProjectRemote(projectPath: string) {
  return useMutation({
    mutationFn: () => unwrap(commands.fetchProjectRemote(projectPath)),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["workstream-bases", projectPath] });
    },
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
    // `skipScripts` runs the archive without the project's Archive script, used
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

/** Archived workstreams across all projects, newest-first, feeds the
 *  Archive history view. Separate query key from the active lists so
 *  archive/restore mutations invalidate both halves cleanly. */
export function useArchivedWorkstreams(enabled = true) {
  return useQuery({
    queryKey: ["workstreams", "@archived"],
    queryFn: () => unwrap(commands.listArchivedWorkstreams()),
    enabled,
  });
}

/** Restore the project checkout's modified tracked files to HEAD, resolving to
 *  how many were reverted. Destructive, the caller confirms first.
 *
 *  Deliberately invalidates nothing: no workstream row changes, and the caller
 *  retries a create straight after. A refetch here would put `useWorkstreams`
 *  in flight during that create, and ProjectPage swaps to a spinner while it
 *  is, unmounting the dialog before the create's `onSuccess` can close it. */
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

// ── Pull request (derived live from gh) ─────────────────────────────────────

/** How long an errored PR probe waits before trying again. One mounted query,
 *  so ~60 calls/hr worst case, the cost of never getting stuck. */
const PR_ERROR_RETRY_MS = 60_000;

/** Refresh a workstream's gh-backed PR queries. They sit under their own root,
 *  outside the `["workstream", id]` prefix `invalidateWorkstream` sweeps: the
 *  rest of that family reads local disk, but these spend a GitHub GraphQL call,
 *  and the worktree watcher sweeps on every file change. Riding the prefix cost
 *  one API call per file an agent wrote, enough to drain the 5000/hr quota,
 *  after which every probe fails and a failed probe reads as "no PR". */
export function invalidateWorkstreamPr(id: string) {
  void queryClient.invalidateQueries({ queryKey: ["workstream-pr", id] });
}

/** The branch's PR (or null), derived live from `gh`. Fetches once on mount;
 *  liveness is push-based after that, the backend PR watcher diffs each poll
 *  and emits `prStatusChanged`, which invalidates this query (see
 *  `src/lib/events.ts`). So the bar advances on its own (checks settling, a
 *  merge landing elsewhere flipping Merge → Archive) without a UI timer.
 *
 *  `pollMs` opts into a temporary timer for the one window the push can't cover
 *  fast enough: a ship turn, where the PR appears mid-turn and the watcher is
 *  in its slow "no PR to track" cadence (see `useWorkstreamGitActions`). The
 *  timer stops itself the moment a PR resolves, so it never outlives its job. */
export function useWorkstreamPr(id: string, enabled = true, pollMs?: number) {
  return useQuery({
    queryKey: ["workstream-pr", id],
    queryFn: () => unwrap(commands.workstreamPrStatus(id)),
    enabled: enabled && id.length > 0,
    // Reuse cached status across rapid workstream switches instead of re-running
    // the `gh` subprocess each time; the backend watcher pushes updates between.
    staleTime: 8000,
    refetchInterval: (query) => {
      // A failed probe has to heal itself, and slowly. React Query keeps the
      // last good data on error, so nothing else would ever correct it: the
      // backend stops polling a PR it has already seen merge, and a stuck error
      // would strand the bar on the pre-merge state. Retrying at `pollMs` here
      // would be worse than not retrying, the usual cause is a spent API quota,
      // and 10s hammering is what spends it.
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

/** Open PRs in a workstream's repo for the "Attach PR" picker, filtered live by
 *  `query` (GitHub search syntax). Enabled only while the picker is open. */
export function useWorkstreamOpenPrs(id: string, query: string, enabled = true) {
  return useQuery({
    queryKey: ["workstream-pr", id, "open", query],
    queryFn: () => unwrap(commands.listWorkstreamOpenPrs(id, query)),
    enabled: enabled && id.length > 0,
    // Each `gh pr list` is a subprocess; keep results briefly so reopening the
    // picker or backspacing to a prior query doesn't re-run it.
    staleTime: 15_000,
  });
}

/** Live preview of the PR a user typed into the attach picker (by number), or
 *  null when no such PR exists, confirms the number before attaching. */
export function useWorkstreamPrPreview(id: string, prNumber: number | null, enabled = true) {
  return useQuery({
    queryKey: ["workstream-pr", id, "preview", prNumber],
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
      invalidateWorkstreamPr(id);
      invalidateWorkstream(id);
    },
  });
}

/** Detach a workstream's manually attached PR, reverting to branch detection.
 *  Takes the id as the mutate variable, not a hook arg: this fires from a context
 *  menu that closes on click, so a closed-over id churns to "" (the menu's
 *  fallback) before the async detach resolves, and `onSuccess` would then
 *  invalidate the empty id's PR key, leaving the PR bar stale until the next
 *  window-focus refetch. Reading `id` from the mutation variable pins it. */
export function useDetachPr() {
  return useMutation({
    mutationFn: (id: string) => unwrap(commands.detachWorkstreamPr(id)),
    onSuccess: (_data, id) => {
      invalidateWorkstreamPr(id);
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
      invalidateWorkstreamPr(id);
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
      invalidateWorkstreamPr(id);
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

/** Contents + mtime of a worktree file; the mtime is the save concurrency token. */
export function useWorktreeFile(id: string, path: string) {
  return useQuery({
    queryKey: ["workstream", id, "file", path],
    queryFn: () => unwrap(commands.worktreeReadFile(id, path)),
    enabled: id.length > 0 && path.length > 0,
  });
}

/** Save a worktree file; on success refresh the file cache in place and invalidate
 *  git status. Stale writes reject with a `file_conflict` AppError. */
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

// ── Onboarding ───────────────────────────────────────────────────────────────

/** AI tool install state, gates the AI step in the onboarding wizard.
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
 *  the path) when an existing repo is already there. Re-checked on each mount,
 *  the user may have just created the dir in another step. */
export function useProbeDefaultThoughts() {
  return useQuery({
    queryKey: ["probe-default-thoughts"],
    queryFn: () => commands.probeDefaultThoughts(),
  });
}

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
      // The copy list changed, re-validate so the project's warning badge
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
 *  window regains focus, so fixing a file on disk and returning to the app
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

/** Drop a workstream/kind's captured output from the cache (the dock's Clear).
 *  Writes the emptied value straight in rather than invalidating: the backend has
 *  forgotten it too, and a refetch would flash the stale log on the way out. */
export function forgetScriptOutput(workstreamId: string, kind: ScriptKind) {
  queryClient.setQueryData(["script-output", workstreamId, kind], null);
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

/** Codex sign-in state AND the models it offers, from one `codex app-server`
 *  spawn. Deliberately ONE query: as two, the tab fired both on mount and the
 *  concurrent app-servers clobbered each other, so a working install rendered as
 *  "not signed in" or "no models". Spawning a process isn't free, so it's only
 *  fetched where the answer is shown (`enabled`) and cached for a few minutes,
 *  `codex login` happens out of band in a terminal, hence the Refresh button. */
export function useCodexProbe(enabled = true) {
  return useQuery({
    queryKey: ["codex-probe"],
    queryFn: () => unwrap(commands.codexProbe()),
    enabled,
    staleTime: 5 * 60_000,
  });
}

/**
 * Fold models the picker has never offered into a curated visible-models list,
 * once per app run, as soon as the catalog is known.
 *
 * Without this a curated list is an allowlist frozen on the day it was edited,
 * and the only way to see a new model is "Reset to default", which also undoes
 * every deliberate removal. The sync is additive, and a no-op for anyone who
 * never curated. Failures are swallowed, since a settings write that didn't
 * stick must not surface as an error in the composer.
 *
 * `catalogSettled` must stay false until the probe finishes either way. The
 * static fallback is never empty, so syncing early would latch against it and
 * never see the live-only keys. A sync against the fallback latches only that
 * tier, so a later refetch of the live catalog still gets its own sync.
 */
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
    // Latch before awaiting: several composers mount at once and would
    // otherwise each fire the same write.
    synced.current = catalogLive ? "live" : "static";
    if (!sync) return;
    void commands
      .claudeSyncModels(sync.add, sync.seen)
      .then(() => queryClient.invalidateQueries({ queryKey: ["bootstrap"] }))
      .catch(() => {});
  }, [settings, catalog, catalogSettled, catalogLive, hasCatalog]);
}

/** The `claude` CLI's own model catalog (`list_models`), which reflects this
 *  install's provider, settings and org policy, the authority on what the
 *  picker can offer. Deduped by react-query across tabs, so many open composers
 *  cost one ~1s probe; callers fall back to the static table while it loads or
 *  if it fails (no CLI, signed out, older CLI without the control request). */
export function useClaudeProbe(enabled = true) {
  return useQuery({
    queryKey: ["claude-probe"],
    queryFn: () => unwrap(commands.claudeProbe()),
    enabled,
    staleTime: 5 * 60_000,
  });
}

/** Sign Codex in with an API key. The key is passed straight through to
 *  `codex login --with-api-key` and never stored by us, Codex owns the
 *  credential (`~/.codex/auth.json`). The command returns a full probe (which
 *  models Codex offers depends on who is signed in), so the tab updates without
 *  a second spawn. */
export function useCodexLoginWithApiKey() {
  return useMutation({
    mutationFn: (apiKey: string) => unwrap(commands.codexLoginWithApiKey(apiKey)),
    onSuccess: (probe) => queryClient.setQueryData(["codex-probe"], probe),
  });
}

/** Sign Codex out (`codex logout`). */
export function useCodexLogout() {
  return useMutation({
    mutationFn: () => unwrap(commands.codexLogout()),
    onSuccess: (probe) => queryClient.setQueryData(["codex-probe"], probe),
  });
}


/** Set up the user's thoughts repo (clone/disk/create) and write config.json.
 *  Invalidates the full bootstrap/projects/docs/workstreams set on success so
 *  the rest of the app reflects the new ready state immediately. */
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
