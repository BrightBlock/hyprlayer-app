import { Icon } from "@/components/Icon";
import { Spinner } from "@/components/ui";
import type { ThoughtsSyncState } from "./useThoughtsSync";

function syncView(sync: ThoughtsSyncState) {
  const { status, syncing, error, flash } = sync;
  const noRemote = status ? !status.hasRemote : false;
  const failed = Boolean(error) || flash === "bad";
  const toPush = (status?.dirty ?? 0) + (status?.ahead ?? 0);
  const toPull = status?.behind ?? 0;

  const pendingParts = [
    toPush > 0 && `${toPush} local change${toPush === 1 ? "" : "s"} to push`,
    toPull > 0 && `${toPull} commit${toPull === 1 ? "" : "s"} to pull`,
  ].filter(Boolean);
  const pending = !syncing && !failed && flash !== "ok" && pendingParts.length > 0;

  const title = syncing
    ? "Syncing thoughts…"
    : flash === "ok"
      ? "Thoughts synced"
      : error
        ? `Thoughts sync failed: ${error}`
        : !status
          ? "Sync thoughts"
          : noRemote
            ? "Thoughts repo has no remote to sync with"
            : pendingParts.length > 0
              ? `${pendingParts.join(" · ")}. Click to sync`
              : "Thoughts synced. Click to sync now";

  const label = syncing
    ? "syncing thoughts"
    : failed
      ? "sync failed"
      : noRemote
        ? "thoughts · no remote"
        : pending
          ? toPush > 0
            ? `${toPush} unsynced`
            : `${toPull} to pull`
          : status || flash === "ok"
            ? "thoughts synced"
            : "thoughts";

  const tone = failed ? "text-bad" : flash === "ok" ? "text-ok" : "text-ink-dim";
  const icon = syncing ? (
    <Spinner size="sm" />
  ) : flash === "ok" ? (
    <Icon name="check" size="sm" />
  ) : (
    <Icon name="sync" size="sm" />
  );
  return { noRemote, pending, title, label, tone, icon };
}

function PendingDot({ className }: { className: string }) {
  return (
    <span className={`absolute h-1.5 w-1.5 rounded-full bg-warn ${className}`} aria-hidden="true" />
  );
}

export function ThoughtsSyncButton({ sync }: { sync: ThoughtsSyncState }) {
  if (sync.hidden) return null;
  const { noRemote, pending, title, tone, icon } = syncView(sync);
  return (
    <button
      type="button"
      onClick={sync.syncNow}
      disabled={sync.syncing || noRemote}
      className={`relative flex h-6 w-6 items-center justify-center rounded ${tone} enabled:hover:bg-edge/60 enabled:hover:text-ink disabled:cursor-default`}
      title={title}
      aria-label="Sync thoughts"
    >
      {icon}
      {pending && <PendingDot className="right-0.5 top-0.5" />}
    </button>
  );
}

/** The status bar's labelled form: says whether new plans and notes have reached the thoughts remote. */
export function ThoughtsSyncStatus({ sync, compact }: { sync: ThoughtsSyncState; compact: boolean }) {
  if (sync.hidden) return null;
  const { noRemote, pending, title, label, tone, icon } = syncView(sync);
  return (
    <button
      type="button"
      onClick={sync.syncNow}
      disabled={sync.syncing || noRemote}
      className={`flex h-6 shrink-0 items-center gap-1.5 rounded-control px-1.5 ${tone} enabled:hover:bg-panel-2 enabled:hover:text-ink disabled:cursor-default focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/60`}
      title={title}
      aria-label={`Sync thoughts: ${title}`}
    >
      <span className="relative flex">
        {icon}
        {pending && <PendingDot className="-right-0.5 -top-0.5" />}
      </span>
      {!compact && <span className="whitespace-nowrap">{label}</span>}
    </button>
  );
}
