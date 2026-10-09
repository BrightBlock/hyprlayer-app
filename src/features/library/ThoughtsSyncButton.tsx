import { useEffect, useRef, useState, type ReactNode } from "react";

import { Icon } from "@/components/Icon";
import { BAR_POPOVER, CliOutput, cx, Popover, Spinner } from "@/components/ui";
import { basename } from "@/lib/path";
import type { ThoughtsSyncFailure, ThoughtsSyncState } from "./useThoughtsSync";

const SHOW_DELAY_MS = 250;
const HIDE_DELAY_MS = 200;

function syncView(sync: ThoughtsSyncState) {
  const { status, syncing, error, flash } = sync;
  const noRemote = status ? !status.hasRemote : false;
  const failed = Boolean(error);
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
        ? `Thoughts sync failed: ${error.message}`
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

function useHoverIntent() {
  const [open, setOpen] = useState(false);
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(timer.current), []);
  const settle = (next: boolean, delay: number) => {
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setOpen(next), delay);
  };
  return {
    open,
    close: () => settle(false, 0),
    onMouseEnter: () => settle(true, open ? 0 : SHOW_DELAY_MS),
    onMouseLeave: () => settle(false, HIDE_DELAY_MS),
  };
}

function FailureCard({ failure }: { failure: ThoughtsSyncFailure }) {
  return (
    <>
      <div className="flex items-baseline gap-2 px-1.5 pt-0.5">
        <span className="lbl shrink-0">Sync failed</span>
        <span className="min-w-0 truncate text-meta text-ink-dim">
          {basename(failure.projectPath)}
        </span>
      </div>
      <CliOutput text={failure.message} />
      {failure.remedy && <p className="mt-1.5 px-1.5 text-ui-sm text-ink-2">{failure.remedy}</p>}
    </>
  );
}

/** Hover lives here, not on the button: WebKit sends a disabled button no mouse events. */
function SyncHoverTarget({
  sync,
  title,
  side,
  className,
  children,
}: {
  sync: ThoughtsSyncState;
  title: string;
  side: "top" | "bottom";
  className?: string;
  children: ReactNode;
}) {
  const anchorRef = useRef<HTMLSpanElement>(null);
  const hover = useHoverIntent();
  return (
    <span
      ref={anchorRef}
      className={cx("flex", className)}
      title={sync.error ? undefined : title}
      onMouseEnter={hover.onMouseEnter}
      onMouseLeave={hover.onMouseLeave}
    >
      {children}
      {sync.error && (
        // React's enter/leave follows the component tree, so the portaled card counts as inside.
        <Popover
          open={hover.open}
          onClose={hover.close}
          anchorRef={anchorRef}
          side={side}
          align="end"
          sideOffset={10}
          caret
          modal={false}
          className={BAR_POPOVER}
        >
          <FailureCard failure={sync.error} />
        </Popover>
      )}
    </span>
  );
}

export function ThoughtsSyncButton({ sync }: { sync: ThoughtsSyncState }) {
  if (sync.hidden) return null;
  const { noRemote, pending, title, tone, icon } = syncView(sync);
  return (
    <SyncHoverTarget sync={sync} title={title} side="bottom">
      <button
        type="button"
        onClick={sync.syncNow}
        disabled={sync.syncing || noRemote}
        className={`relative flex h-6 w-6 items-center justify-center rounded ${tone} enabled:hover:bg-edge/60 enabled:hover:text-ink disabled:pointer-events-none`}
        aria-label={`Sync thoughts: ${title}`}
      >
        {icon}
        {pending && <PendingDot className="right-0.5 top-0.5" />}
      </button>
    </SyncHoverTarget>
  );
}

/** The status bar's labelled form: says whether new plans and notes have reached the thoughts remote. */
export function ThoughtsSyncStatus({ sync, compact }: { sync: ThoughtsSyncState; compact: boolean }) {
  if (sync.hidden) return null;
  const { noRemote, pending, title, label, tone, icon } = syncView(sync);
  return (
    <SyncHoverTarget sync={sync} title={title} side="top" className="shrink-0">
      <button
        type="button"
        onClick={sync.syncNow}
        disabled={sync.syncing || noRemote}
        className={`flex h-6 shrink-0 items-center gap-1.5 rounded-control px-1.5 ${tone} enabled:hover:bg-panel-2 enabled:hover:text-ink disabled:pointer-events-none focus:outline-none focus-visible:bg-panel-2 focus-visible:text-ink`}
        aria-label={`Sync thoughts: ${title}`}
      >
        <span className="relative flex">
          {icon}
          {pending && <PendingDot className="-right-0.5 -top-0.5" />}
        </span>
        {!compact && <span className="whitespace-nowrap">{label}</span>}
      </button>
    </SyncHoverTarget>
  );
}
