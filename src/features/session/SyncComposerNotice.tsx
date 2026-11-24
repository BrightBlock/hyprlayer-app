import { useEffect, useState } from "react";

import { Icon } from "@/components/Icon";
import { Button, Skeleton, Spinner } from "@/components/ui";
import { formatRelativeTime } from "@/lib/format";
import { deviceLabel, type HeldView, type SyncState } from "./transcriptSync";

type SyncNotice =
  | { kind: "behind"; count: number; device: string; at: number | null }
  | { kind: "retry"; count: number; seconds: number }
  | { kind: "syncing"; count: number }
  | { kind: "synced"; device: string }
  | { kind: "guard"; count: number; device: string; syncing: boolean }
  | { kind: "fallback"; loading: boolean };

/** Revisions this view hasn't shown; the same count a refusal's `missing_turns` reports. */
export function pendingCount(sync: SyncState): number {
  return Math.max(0, sync.remoteRev - sync.localRev);
}

/** `deviceLabel` at the start of a sentence: only the fallback needs a capital. */
export function sentenceDevice(sync: SyncState): string {
  const device = deviceLabel(sync);
  return device === "another device" ? "Another device" : device;
}

const NO_HELD: HeldView = { at: null, dismissedRev: null };

export function syncNotice(
  sync: SyncState | undefined,
  drifted: boolean,
  loading: boolean,
  terminal = false,
  held: HeldView = NO_HELD,
): SyncNotice | null {
  // Only a server without sync marks a view drifted; an ended view may have been resumed elsewhere.
  if (drifted) return { kind: "fallback", loading };
  if (!sync || sync.gone || terminal) return null;
  const count = pendingCount(sync);
  // A pull started from the guard keeps the card, so the draft's warning doesn't flicker away.
  if (sync.phase === "guard" || (sync.phase === "syncing" && sync.draft && count > 0)) {
    return { kind: "guard", count, device: sentenceDevice(sync), syncing: sync.phase === "syncing" };
  }
  if (sync.phase === "behind" && sync.retry) {
    return { kind: "retry", count, seconds: Math.round(sync.retry.delayMs / 1000) };
  }
  // Count 0 is held items with no new revision, such as a turn end or a prompt.
  if (sync.phase === "behind") {
    if (held.dismissedRev !== null && sync.remoteRev <= held.dismissedRev) return null;
    return { kind: "behind", count, device: deviceLabel(sync), at: held.at };
  }
  if (sync.phase === "syncing" && count > 0) return { kind: "syncing", count };
  if (sync.syncedWith) return { kind: "synced", device: sync.syncedWith };
  return null;
}

const turns = (n: number) => `${n} ${n === 1 ? "turn" : "turns"}`;
// Held items with no new revision have nothing to count.
const pullLabel = (n: number) => (n > 0 ? `Pull ${turns(n)}` : "Pull");

const CLOSE =
  "flex h-5 w-5 shrink-0 items-center justify-center rounded-control text-ink-dim hover:bg-edge/60 hover:text-ink focus:outline-none focus-visible:bg-edge/60 focus-visible:text-ink";

function Age({ at }: { at: number }) {
  const [, tick] = useState(0);
  useEffect(() => {
    const id = setInterval(() => tick((n) => n + 1), 30_000);
    return () => clearInterval(id);
  }, []);
  return <> · {formatRelativeTime(at)}</>;
}

const DOT = "h-1.5 w-1.5 shrink-0 rounded-full";
const LINE = "mb-2 flex items-center gap-1.5 px-1 text-meta text-ink-dim";
const LINK =
  "rounded-control px-1 font-mono text-meta text-info hover:bg-info/10 focus:outline-none focus-visible:bg-info/10";

export function SyncComposerNotice({
  notice,
  onPull,
  onSendAnyway,
  onLoad,
  onDismiss,
}: {
  notice: SyncNotice | null;
  onPull: () => void;
  onSendAnyway: () => void;
  onLoad: () => void;
  onDismiss: () => void;
}) {
  // Always mounted: a live region added together with its text is often not announced.
  return (
    <div role="status" aria-live="polite">
      {notice && <NoticeBody notice={notice} onPull={onPull} onSendAnyway={onSendAnyway} onLoad={onLoad} onDismiss={onDismiss} />}
    </div>
  );
}

function NoticeBody({
  notice,
  onPull,
  onSendAnyway,
  onLoad,
  onDismiss,
}: {
  notice: SyncNotice;
  onPull: () => void;
  onSendAnyway: () => void;
  onLoad: () => void;
  onDismiss: () => void;
}) {
  switch (notice.kind) {
    case "behind":
      return (
        <div className="flex items-center gap-2 rounded-t-lg border-b border-info/30 bg-info/10 py-1.5 pl-3 pr-1.5 text-ui text-ink-dim">
          <span className={`${DOT} bg-info`} aria-hidden />
          <span className="min-w-0 flex-1 truncate">
            {notice.count > 0 ? (
              <>
                <span className="font-semibold text-ink">{`${notice.count} new ${notice.count === 1 ? "turn" : "turns"}`}</span> from{" "}
              </>
            ) : (
              "New from "
            )}
            <span className="font-mono text-ink">{notice.device}</span>
            {notice.at !== null && <Age at={notice.at} />}
          </span>
          <Button variant="primary" size="sm" onClick={onPull}>
            <Icon name="arrowDown" size="xs" />
            Load
          </Button>
          <button type="button" onClick={onDismiss} aria-label="Dismiss" className={CLOSE}>
            <Icon name="close" size="xs" />
          </button>
        </div>
      );
    case "retry":
      return (
        <div className={LINE}>
          <span className={`${DOT} bg-info`} aria-hidden />
          <span>
            Couldn't pull {notice.count > 0 ? turns(notice.count) : "new turns"} · retrying in {notice.seconds}s
          </span>
          <button type="button" onClick={onPull} className={LINK}>
            now
          </button>
        </div>
      );
    case "syncing":
      return (
        <div className={LINE}>
          <Spinner size="xs" />
          <span>Syncing {turns(notice.count)} · keep typing, send waits</span>
        </div>
      );
    case "synced":
      return (
        <div className={`${LINE} animate-sync-in`}>
          <Icon name="check" size="xs" className="text-info" />
          <span>Synced with {notice.device}</span>
        </div>
      );
    case "guard":
      return (
        <div className="mb-2 rounded-md border border-warn/40 bg-warn/5 px-3 py-2.5">
          <div className="flex items-center gap-2">
            <span className={`${DOT} bg-warn`} aria-hidden />
            <span className="text-ui font-semibold text-ink">This session moved on from another client</span>
          </div>
          <p className="mt-1 text-ui text-ink-dim">
            {notice.device} added {notice.count > 0 ? turns(notice.count) : "to this session"} while you were drafting.
            Pull them in first so your message answers the latest state. Your draft stays.
          </p>
          <div className="mt-2.5 flex items-center gap-2">
            <Button variant="primary" size="sm" onClick={onPull} disabled={notice.syncing}>
              {notice.syncing ? (
                <span className="flex w-3 flex-col gap-0.5" aria-hidden>
                  <Skeleton shimmer className="h-0.5" />
                  <Skeleton shimmer className="h-0.5 w-2/3" />
                </span>
              ) : (
                <Icon name="arrowDown" size="xs" />
              )}
              {pullLabel(notice.count)}
            </Button>
            <Button variant="ghost" size="sm" onClick={onSendAnyway}>
              Send anyway
            </Button>
            <span className="ml-auto text-meta text-ink-dim">draft kept · ⏎ send</span>
          </div>
        </div>
      );
    case "fallback":
      return (
        <div className="mb-2 flex items-center gap-2 rounded-md border border-info/40 bg-info/10 px-3 py-1.5 text-ui">
          {notice.loading ? (
            <>
              <Spinner size="xs" />
              <span className="flex-1 text-ink-dim">Loading turns…</span>
            </>
          ) : (
            <>
              <span className={`${DOT} bg-info`} aria-hidden />
              <span className="flex-1 text-ink">New turns from another device</span>
              <button
                type="button"
                onClick={onLoad}
                className="rounded-control px-1.5 py-0.5 font-medium text-info hover:bg-info/10 focus:outline-none focus-visible:bg-info/10"
              >
                Load
              </button>
              <button type="button" onClick={onDismiss} aria-label="Dismiss" className={CLOSE}>
                <Icon name="close" size="xs" />
              </button>
            </>
          )}
        </div>
      );
  }
}
