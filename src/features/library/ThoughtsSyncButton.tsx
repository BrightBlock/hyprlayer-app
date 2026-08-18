import { CheckIcon, SyncIcon } from "@/components/icons";
import { Spinner } from "@/components/ui";
import type { ThoughtsSyncState } from "./useThoughtsSync";

/** Library-header mirror of `hyprlayer thoughts sync`; surfaces repo state and
 *  covers the gap between the Library's auto open/re-scope syncs. The icon flashes
 *  the verdict tone after each attempt, a quiet success is otherwise invisible. */
export function ThoughtsSyncButton({ sync }: { sync: ThoughtsSyncState }) {
  if (sync.hidden) return null;

  const { status, syncing, error, flash } = sync;
  const noRemote = status ? !status.hasRemote : false;
  const failed = Boolean(error) || flash === "bad";
  const toPush = (status?.dirty ?? 0) + (status?.ahead ?? 0);
  const toPull = status?.behind ?? 0;

  const pendingParts = [
    toPush > 0 && `${toPush} local change${toPush === 1 ? "" : "s"} to push`,
    toPull > 0 && `${toPull} commit${toPull === 1 ? "" : "s"} to pull`,
  ].filter(Boolean);

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

  const tone = failed ? "text-bad" : flash === "ok" ? "text-ok" : "text-ink-dim";

  return (
    <button
      type="button"
      onClick={sync.syncNow}
      disabled={syncing || noRemote}
      className={`relative flex h-6 w-6 items-center justify-center rounded ${tone} enabled:hover:bg-edge/60 enabled:hover:text-ink disabled:cursor-default`}
      title={title}
      aria-label="Sync thoughts"
    >
      {syncing ? (
        <Spinner size={12} />
      ) : flash === "ok" ? (
        <CheckIcon size={13} />
      ) : (
        <SyncIcon size={13} />
      )}
      {!syncing && !failed && flash !== "ok" && pendingParts.length > 0 && (
        <span
          className="absolute right-0.5 top-0.5 h-1.5 w-1.5 rounded-full bg-warn"
          aria-hidden="true"
        />
      )}
    </button>
  );
}
