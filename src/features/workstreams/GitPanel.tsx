import { useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";

import type { WorkstreamView } from "@/lib/bindings";
import {
  useBootstrap,
  useFetchWorkstreamRemote,
  useSetWorkstreamBase,
  useUpdateFromBase,
  useWorkstreamBases,
} from "@/lib/queries";
import { IS_MACOS } from "@/lib/platform";
import { formatChord } from "@/lib/keybindings/chord";
import { effectiveChord } from "@/lib/keybindings/resolve";
import { ErrorNote, IconButton, Spinner } from "@/components/ui";
import {
  ChevronDownIcon,
  GitCommitIcon,
  GitPullRequestIcon,
  SplitIcon,
} from "@/components/icons";
import { useWorkstreamGitActions } from "./useGitActions";
import { AheadBehind } from "./AheadBehind";
import { PrStatusBar } from "./PrStatusBar";
import { AttachPrDialog } from "./AttachPrDialog";
import { ArchiveWorkstreamDialog } from "./ArchiveWorkstreamDialog";

/** Circular-arrow refresh glyph for the branch-list reload control. */
function RefreshIcon({ size = 12 }: { size?: number }) {
  return (
    <svg aria-hidden="true" width={size} height={size} viewBox="0 0 24 24" fill="none">
      <path
        d="M20 12a8 8 0 1 1-2.34-5.66M20 4v4h-4"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** Arrow-down-to-a-line glyph for the "pull base commits in" action. */
function PullIcon({ size = 13 }: { size?: number }) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M12 4v11" />
      <path d="m7 10 5 5 5-5" />
      <path d="M5 20h14" />
    </svg>
  );
}

/** Drop the `origin/` remote prefix for display — the stored base is a git rev
 *  (`origin/main`) but users (and GitHub) think in branch names (`main`). */
function shortBranch(ref: string): string {
  return ref.replace(/^origin\//, "");
}

/** The branch → base row inside the PULL REQUEST card. The base is a functional
 *  dropdown: picking a branch re-targets the merge destination (and re-points an
 *  open PR via `gh pr edit`), so git-flow teams can merge into `develop`. */
function BranchTargetRow({ view }: { view: WorkstreamView }) {
  const [open, setOpen] = useState(false);
  const setBase = useSetWorkstreamBase(view.item.id);
  // Branch list fetched lazily — only while the menu is open.
  const bases = useWorkstreamBases(view.item.projectPath, open);
  // The list is built from the local repo's remote-tracking refs, so deleting a
  // branch on the remote doesn't change it until a `git fetch --prune`. Refresh
  // therefore prunes first (network), THEN re-scans — otherwise it just re-reads
  // the same stale `origin/*` refs and the deleted branches linger.
  const fetchRemote = useFetchWorkstreamRemote(view.item.id);
  const refreshBranches = async () => {
    await fetchRemote.mutateAsync().catch(() => {}); // offline: still re-scan local
    await bases.refetch();
  };
  const refreshing = fetchRemote.isPending || bases.isFetching;
  // A PR base must exist on the remote, so offer only remote-tracking branches
  // (`origin/*`). A local-only branch — e.g. another workstream's unpushed
  // feature branch — isn't on GitHub, so `gh pr edit --base` rejects it with
  // "Proposed base branch … was not found". The full `origin/<name>` ref is
  // stored (it revparse-resolves locally for ahead/behind and is stripped for
  // gh + the Ship prompt); the menu shows it without the prefix.
  const baseShort = shortBranch(view.item.baseBranch);
  const choices = (bases.data?.branches ?? [])
    .filter((b) => b.startsWith("origin/"))
    .map((b) => ({ ref: b, label: shortBranch(b) }))
    .filter((c) => c.label !== view.item.branch);

  return (
    <div className="space-y-1.5">
      <div className="flex items-center gap-1.5">
        {/* Source branch — fixed, this workstream's own branch. */}
        <span
          className="inline-flex min-w-0 flex-1 items-center gap-1.5 rounded-md border border-edge bg-panel px-2 py-1"
          title={view.item.branch}
        >
          <SplitIcon size={13} className="shrink-0 text-ok" />
          <span className="min-w-0 truncate font-mono text-xs text-ink">{view.item.branch}</span>
        </span>
        <span className="shrink-0 text-ink-dim">→</span>
        {/* Target (base) branch — the functional picker. */}
        <div className="relative shrink-0">
          <button
            type="button"
            onClick={() => setOpen((o) => !o)}
            disabled={setBase.isPending}
            className="inline-flex items-center gap-1 rounded-md border border-edge bg-panel px-2 py-1 font-mono text-xs text-ink hover:bg-edge/50 disabled:opacity-60"
            title={`Merge target: ${view.item.baseBranch} — click to change`}
            aria-label="Change target branch"
          >
            {setBase.isPending && <Spinner size={11} />}
            <span className="max-w-[110px] truncate">{baseShort}</span>
            <ChevronDownIcon size={11} className="text-ink-dim" />
          </button>
          {open && (
            <>
              {/* Backdrop closes the menu on any outside click. */}
              <div className="fixed inset-0 z-40" onClick={() => setOpen(false)} />
              <div className="absolute right-0 top-full z-50 mt-1 w-56 rounded-md border border-edge bg-panel p-1 shadow-2xl">
                {/* Header + refresh: re-runs the branch scan so a branch added
                    since the list was cached (staleTime 30s) shows up on demand. */}
                <div className="flex items-center justify-between gap-2 px-2 py-1">
                  <span className="text-[10px] font-semibold uppercase tracking-wider text-ink-dim">
                    Target branch
                  </span>
                  <button
                    type="button"
                    onClick={() => void refreshBranches()}
                    disabled={refreshing}
                    className="flex h-5 w-5 items-center justify-center rounded text-ink-dim hover:bg-edge/70 hover:text-ink disabled:opacity-50"
                    title="Fetch &amp; prune, then refresh branches"
                    aria-label="Refresh branches"
                  >
                    {refreshing ? <Spinner size={11} /> : <RefreshIcon size={12} />}
                  </button>
                </div>
                <div className="max-h-60 overflow-y-auto">
                  {bases.isLoading ? (
                    <div className="flex justify-center py-3">
                      <Spinner size={13} />
                    </div>
                  ) : choices.length === 0 ? (
                    <div className="px-2 py-1.5 text-xs text-ink-dim">
                      No remote branches{refreshing ? "" : " — try refreshing"}.
                    </div>
                  ) : (
                    choices.map((c) => (
                      <button
                        key={c.ref}
                        type="button"
                        onClick={() => {
                          setOpen(false);
                          if (c.ref !== view.item.baseBranch) setBase.mutate(c.ref);
                        }}
                        className={`flex w-full items-center gap-2 rounded px-2 py-1.5 text-left font-mono text-xs hover:bg-edge/60 hover:text-ink ${
                          c.label === baseShort ? "text-ink" : "text-ink-dim"
                        }`}
                      >
                        <SplitIcon size={12} className="shrink-0" />
                        <span className="min-w-0 flex-1 truncate">{c.label}</span>
                        {c.label === baseShort && (
                          <span className="shrink-0 text-[10px] text-ink-dim/70">current</span>
                        )}
                      </button>
                    ))
                  )}
                </div>
              </div>
            </>
          )}
        </div>
      </div>
      {setBase.isError && <ErrorNote error={setBase.error} />}
    </div>
  );
}

/**
 * The GIT section at the top of the changes sidebar (the mockup): ahead/behind,
 * a PULL REQUEST card (source → target picker + either an attach-PR link or the
 * live PR's controls), and the Commit / Ship buttons. Consolidates every git
 * action here so the app header carries none. Renders a bare "GIT" band when no
 * workstream is selected. Owns its own Attach-PR + Archive dialogs.
 */
export function GitPanel({ view }: { view: WorkstreamView | null }) {
  const navigate = useNavigate();
  const actions = useWorkstreamGitActions(view);
  // Effective chords for the button tooltips — these run the same commands.
  const kbOverrides = useBootstrap().data?.settings.keybindings?.overrides ?? {};
  const chordHint = (commandId: string): string => {
    const chord = effectiveChord(kbOverrides, commandId, IS_MACOS);
    return chord ? ` (${formatChord(chord, IS_MACOS)})` : "";
  };
  const [attachTarget, setAttachTarget] = useState<WorkstreamView | null>(null);
  const [archiveTarget, setArchiveTarget] = useState<WorkstreamView | null>(null);
  // "Update from base" lives here (not a child) so the small pull button and its
  // full-width conflict/error message share one mutation. Reset on workstream
  // switch so a prior branch's result doesn't bleed into the next.
  const pull = useUpdateFromBase(view?.item.id ?? "");
  useEffect(() => {
    pull.reset();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [view?.item.id]);
  const behind = view?.git.behind ?? 0;
  const conflicts = pull.data?.kind === "conflicts" ? pull.data : null;
  // Fetch the remote on demand so ahead/behind + PR reflect remote branch changes.
  const refresh = useFetchWorkstreamRemote(view?.item.id ?? "");

  return (
    <>
      <div className="shrink-0 border-b border-edge">
        <div className="flex h-9 items-center justify-between px-3">
          <span className="text-xs font-semibold uppercase tracking-wide text-ink-dim">Git</span>
          {view && (
            <div className="flex items-center gap-1.5">
              <AheadBehind ahead={view.git.ahead} behind={view.git.behind} />
              <IconButton
                onClick={() => refresh.mutate()}
                disabled={refresh.isPending}
                title="Fetch remote — refresh ahead/behind and PR status"
                aria-label="Fetch remote"
              >
                {refresh.isPending ? <Spinner size={14} /> : <RefreshIcon size={14} />}
              </IconButton>
            </div>
          )}
        </div>

        {view && (
          <div className="space-y-2.5 px-3 pb-3">
            {/* PULL REQUEST card. */}
            <div className="space-y-2 rounded-lg border border-edge bg-panel-2 p-2.5">
              <div className="text-[10px] font-semibold uppercase tracking-wider text-ink-dim">
                Pull Request
              </div>
              <BranchTargetRow view={view} />
              {/* Footer: the attach-PR link (or live-PR controls) on the left, and
                  a small "pull base commits in" arrow on the right when behind. */}
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0 flex-1">
                  {actions.hasLivePr ? (
                    <PrStatusBar view={view} onArchive={setArchiveTarget} />
                  ) : (
                    <button
                      type="button"
                      onClick={() => setAttachTarget(view)}
                      className="text-xs font-medium text-accent hover:underline"
                    >
                      Attach an existing PR…
                    </button>
                  )}
                </div>
                {behind > 0 && !conflicts && (
                  <button
                    type="button"
                    disabled={pull.isPending}
                    onClick={() => pull.mutate()}
                    className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-edge bg-panel text-ink-dim hover:bg-edge/50 hover:text-ink disabled:opacity-60"
                    title={`Update from ${view.item.baseBranch} — pull ${behind} new commit${
                      behind === 1 ? "" : "s"
                    } in`}
                    aria-label={`Update from ${view.item.baseBranch}`}
                  >
                    {pull.isPending ? <Spinner size={12} /> : <PullIcon size={14} />}
                  </button>
                )}
              </div>
              {conflicts && (
                <p className="rounded-md border border-bad/40 bg-bad/10 px-2 py-1.5 text-[11px] leading-relaxed text-bad">
                  Merge conflicts in {conflicts.files.length} file
                  {conflicts.files.length === 1 ? "" : "s"} — resolve them in the Changes panel
                  below, then commit.
                </p>
              )}
              {pull.data?.kind === "upToDate" && (
                <p className="px-1 text-[11px] text-ink-dim">Already up to date.</p>
              )}
              {/* Dismiss clears the mutation back to idle; re-running the local
                  merge (the pull button) re-throws and re-shows it on failure. */}
              {pull.isError && <ErrorNote error={pull.error} onDismiss={() => pull.reset()} />}
            </div>

            {/* Commit / Ship. Both gate on the live changed-count; neither shows
                when there's nothing to do (clean tree + live PR). */}
            {(actions.canCommit || actions.canShip) && (
              <div className="flex gap-2">
                {actions.canCommit && (
                  <button
                    type="button"
                    disabled={actions.committing}
                    onClick={actions.doCommit}
                    className="inline-flex h-9 flex-1 items-center justify-center gap-2 rounded-md border border-edge bg-panel-2 text-sm font-medium text-ink hover:bg-edge/50 disabled:opacity-60"
                    title={`Runs the Commit prompt — commits locally, then asks whether to push${chordHint("ws.commit")}`}
                  >
                    <GitCommitIcon size={15} />
                    Commit · {actions.changedCount}
                  </button>
                )}
                {actions.canShip && (
                  <button
                    type="button"
                    disabled={actions.shipping}
                    onClick={actions.doShip}
                    className="inline-flex h-9 flex-1 items-center justify-center gap-2 rounded-md bg-accent text-sm font-medium text-white hover:bg-accent/90 disabled:opacity-60"
                    title={`Runs the Ship prompt — commit, push, and open a pull request${chordHint("ws.ship")}`}
                  >
                    <GitPullRequestIcon size={15} />
                    Ship
                  </button>
                )}
              </div>
            )}
          </div>
        )}
      </div>

      <AttachPrDialog target={attachTarget} onClose={() => setAttachTarget(null)} />
      <ArchiveWorkstreamDialog
        target={archiveTarget}
        onClose={() => setArchiveTarget(null)}
        onArchived={() => {
          // Drop back to the project board — the archived worktree is gone.
          const archived = archiveTarget;
          setArchiveTarget(null);
          if (archived)
            void navigate({
              to: "/projects/$projectPath",
              params: { projectPath: archived.item.projectPath },
              search: {},
            });
        }}
      />
    </>
  );
}
