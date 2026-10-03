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
import { isDetachedHead, shortBranch } from "@/lib/domain/projects";
import { IS_MACOS } from "@/lib/platform";
import { formatChord } from "@/lib/keybindings/chord";
import { effectiveChord } from "@/lib/keybindings/resolve";
import { ErrorNote, IconButton, Spinner } from "@/components/ui";
import { Icon } from "@/components/Icon";
import { useWorkstreamGitActions } from "./useGitActions";
import { AheadBehind } from "./AheadBehind";
import { PrStatusBar } from "./PrStatusBar";
import { AttachPrDialog } from "./AttachPrDialog";
import { ArchiveWorkstreamDialog } from "./ArchiveWorkstreamDialog";

function BranchTargetRow({ view }: { view: WorkstreamView }) {
  const [open, setOpen] = useState(false);
  const setBase = useSetWorkstreamBase(view.item.id);
  const bases = useWorkstreamBases(view.item.projectPath, open);
  const fetchRemote = useFetchWorkstreamRemote(view.item.id);
  const refreshBranches = async () => {
    await fetchRemote.mutateAsync().catch(() => {});
    await bases.refetch();
  };
  const refreshing = fetchRemote.isPending || bases.isFetching;
  const baseShort = shortBranch(view.item.baseBranch);
  const detached = isDetachedHead(view);
  const choices = (bases.data?.branches ?? [])
    .filter((b) => b.startsWith("origin/"))
    .map((b) => ({ ref: b, label: shortBranch(b) }))
    .filter((c) => c.label !== view.item.branch);

  return (
    <div className="space-y-1.5">
      <div className="flex items-center gap-1.5">
        <span
          className="inline-flex min-w-0 flex-1 items-center gap-1.5 rounded-md border border-edge bg-panel px-2 py-1"
          title={
            detached
              ? `${view.item.branch} , HEAD is detached (rebase or bisect in progress)`
              : `${view.item.branch} → ${baseShort}`
          }
        >
          <Icon name="branch" size="xs" className={detached ? "text-warn-ink" : "text-ok"} />
          <span className="min-w-0 truncate text-meta text-ink">
            {detached ? "detached HEAD" : view.item.branch}
          </span>
        </span>
        <span className="shrink-0 text-ink-dim">→</span>
        <div className="relative shrink-0">
          <button
            type="button"
            onClick={() => setOpen((o) => !o)}
            disabled={setBase.isPending}
            className="inline-flex items-center gap-1 rounded-md border border-edge bg-panel px-2 py-1 text-meta text-ink hover:bg-edge/50 disabled:opacity-60"
            title={`Merge target: ${view.item.baseBranch} , click to change`}
            aria-label="Change target branch"
          >
            {setBase.isPending && <Spinner size="xs" />}
            <span className="max-w-[110px] truncate">{baseShort}</span>
            <Icon name="chevronDown" size="xs" className="text-ink-dim" />
          </button>
          {open && (
            <>
              <div className="fixed inset-0 z-40" onClick={() => setOpen(false)} />
              <div className="absolute right-0 top-full z-50 mt-1 w-56 rounded-md border border-edge bg-panel p-1 shadow-2xl">
                <div className="flex items-center justify-between gap-2 px-2 py-1">
                  <span className="text-label text-ink-dim">
                    Target branch
                  </span>
                  <button
                    type="button"
                    onClick={() => void refreshBranches()}
                    disabled={refreshing}
                    className="flex h-5 w-5 items-center justify-center rounded text-ink-dim hover:bg-panel-2 hover:text-ink disabled:opacity-50"
                    title="Fetch &amp; prune, then refresh branches"
                    aria-label="Refresh branches"
                  >
                    {refreshing ? <Spinner size="sm" /> : <Icon name="sync" size="sm" />}
                  </button>
                </div>
                <div className="max-h-60 overflow-y-auto">
                  {bases.isLoading ? (
                    <div className="flex justify-center py-3">
                      <Spinner size="sm" />
                    </div>
                  ) : choices.length === 0 ? (
                    <div className="px-2 py-1.5 text-ui text-ink-dim">
                      No remote branches{refreshing ? "" : ", try refreshing"}.
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
                        className={`flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-meta hover:bg-edge/50 hover:text-ink ${
                          c.label === baseShort ? "text-ink" : "text-ink-dim"
                        }`}
                      >
                        <Icon name="branch" size="sm" />
                        <span className="min-w-0 flex-1 truncate">{c.label}</span>
                        {c.label === baseShort && (
                          <span className="shrink-0 text-ui-sm text-ink-dim/70">current</span>
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
      {setBase.isError && <ErrorNote error={setBase.error} onDismiss={() => setBase.reset()} />}
    </div>
  );
}

export function GitPanel({ view }: { view: WorkstreamView | null }) {
  const navigate = useNavigate();
  const actions = useWorkstreamGitActions(view);
  const kbOverrides = useBootstrap().data?.settings.keybindings?.overrides ?? {};
  const chordHint = (commandId: string): string => {
    const chord = effectiveChord(kbOverrides, commandId, IS_MACOS);
    return chord ? ` (${formatChord(chord, IS_MACOS)})` : "";
  };
  const [attachTarget, setAttachTarget] = useState<WorkstreamView | null>(null);
  const [archiveTarget, setArchiveTarget] = useState<WorkstreamView | null>(null);
  const pull = useUpdateFromBase(view?.item.id ?? "");
  useEffect(() => {
    pull.reset();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [view?.item.id]);
  const behind = view?.git.behind ?? 0;
  const conflicts = pull.data?.kind === "conflicts" ? pull.data : null;
  const refresh = useFetchWorkstreamRemote(view?.item.id ?? "");

  return (
    <>
      <div className="shrink-0 border-b border-edge">
        <div className="flex h-9 items-center justify-between px-3">
          <span className="text-label text-ink-dim">Git</span>
          {view && (
            <div className="flex items-center gap-1.5">
              <AheadBehind ahead={view.git.ahead} behind={view.git.behind} />
              <IconButton
                onClick={() => refresh.mutate()}
                disabled={refresh.isPending}
                title="Fetch remote. Refreshes ahead/behind and PR status"
                aria-label="Fetch remote"
              >
                {refresh.isPending ? <Spinner size="sm" /> : <Icon name="sync" size="sm" />}
              </IconButton>
            </div>
          )}
        </div>

        {view && (
          <div className="space-y-2.5 px-3 pb-3">
            <div className="space-y-2 rounded-lg border border-edge bg-panel-2 p-2.5">
              <div className="text-label text-ink-dim">
                Pull Request
              </div>
              <BranchTargetRow view={view} />
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0 flex-1">
                  {actions.hasLivePr ? (
                    <PrStatusBar view={view} onArchive={setArchiveTarget} />
                  ) : (
                    <button
                      type="button"
                      onClick={() => setAttachTarget(view)}
                      className="text-ui font-medium text-accent hover:underline"
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
                    title={`Update from ${view.item.baseBranch} , pulling ${behind} new commit${
                      behind === 1 ? "" : "s"
                    } in`}
                    aria-label={`Update from ${view.item.baseBranch}`}
                  >
                    {pull.isPending ? <Spinner size="sm" /> : <Icon name="pull" size="sm" />}
                  </button>
                )}
              </div>
              {conflicts && (
                <p className="rounded-md border border-bad/40 bg-bad/10 px-2 py-1.5 text-ui-sm leading-prose text-bad">
                  Merge conflicts in {conflicts.files.length} file
                  {conflicts.files.length === 1 ? "" : "s"} , resolve them in the Changes panel
                  below, then commit.
                </p>
              )}
              {pull.data?.kind === "upToDate" && (
                <p className="px-1 text-ui-sm text-ink-dim">Already up to date.</p>
              )}
              {pull.isError && <ErrorNote error={pull.error} onDismiss={() => pull.reset()} />}
            </div>

            {(actions.canCommit || actions.canShip || actions.shipBlocked) && (
              <div className="flex gap-2">
                {actions.canCommit && (
                  <button
                    type="button"
                    disabled={actions.committing}
                    onClick={actions.doCommit}
                    className="inline-flex h-9 flex-1 items-center justify-center gap-2 rounded-md border border-edge bg-panel-2 text-body font-medium text-ink hover:bg-edge/50 disabled:opacity-60"
                    title={`Runs the Commit prompt. Commits locally, then asks whether to push${chordHint("ws.commit")}`}
                  >
                    <Icon name="commit" />
                    Commit · {actions.changedCount}
                  </button>
                )}
                {(actions.canShip || actions.shipBlocked) && (
                  <button
                    type="button"
                    disabled={actions.shipping || actions.shipBlocked !== null}
                    onClick={actions.doShip}
                    className="inline-flex h-9 flex-1 items-center justify-center gap-2 rounded-md bg-accent text-body font-medium text-accent-ink hover:bg-accent-hover disabled:opacity-60"
                    title={
                      actions.shipBlocked ??
                      `Runs the Ship prompt. Commits, pushes, and opens a pull request${chordHint("ws.ship")}`
                    }
                  >
                    <Icon name="pullRequest" />
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
