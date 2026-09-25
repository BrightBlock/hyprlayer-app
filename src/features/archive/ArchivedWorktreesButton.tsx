import { useEffect, useRef } from "react";

import { useConnectionStatus } from "@/lib/queries";
import { Icon } from "@/components/Icon";
import { cx } from "@/components/ui";

import { ArchivedWorktreesPopover } from "./ArchivedWorktreesPopover";
import {
  closeArchivedWorktrees,
  toggleArchivedWorktrees,
  useArchivedWorktrees,
} from "./archivedWorktreesStore";

/** The header's archive button; owns the Archived worktrees popover. */
export function ArchivedWorktreesButton() {
  const anchorRef = useRef<HTMLButtonElement>(null);
  const open = useArchivedWorktrees((s) => s.open);
  const status = useConnectionStatus().data;
  // While disconnected the blocking HostsDialog owns the screen; nothing may stack over it.
  const disconnected = status !== undefined && status.state !== "connected";
  const shown = open && !disconnected;

  useEffect(() => {
    if (open && disconnected) closeArchivedWorktrees();
  }, [open, disconnected]);

  return (
    <>
      <button
        ref={anchorRef}
        type="button"
        onClick={toggleArchivedWorktrees}
        aria-label="Archived worktrees"
        title="Archived worktrees"
        aria-haspopup="dialog"
        aria-expanded={shown}
        className={cx(
          "inline-flex h-[26px] w-[26px] shrink-0 items-center justify-center rounded-control text-ink-dim",
          "hover:bg-edge/60 hover:text-ink focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/60",
          shown && "bg-accent-soft text-ink hover:bg-accent-soft",
        )}
      >
        <Icon name="archive" />
      </button>
      <ArchivedWorktreesPopover open={shown} onClose={closeArchivedWorktrees} anchorRef={anchorRef} />
    </>
  );
}
