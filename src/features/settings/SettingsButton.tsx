import { useEffect, useRef } from "react";

import { useConnectionStatus } from "@/lib/queries";
import { Icon } from "@/components/Icon";
import { cx } from "@/components/ui";
import { ArchivedWorktreesPopover } from "@/features/archive/ArchivedWorktreesPopover";
import {
  closeArchivedWorktrees,
  useArchivedWorktrees,
} from "@/features/archive/archivedWorktreesStore";

import { SettingsPopover } from "./SettingsPopover";
import {
  closeSettingsPopover,
  toggleSettingsPopover,
  useSettingsPopover,
} from "./settingsPopoverStore";

/** The header's gear; owns the Settings popover and its Archived worktrees submenu. */
export function SettingsButton() {
  const anchorRef = useRef<HTMLButtonElement>(null);
  const open = useSettingsPopover((s) => s.open);
  const archivedOpen = useArchivedWorktrees((s) => s.open);
  // While disconnected the blocking HostsDialog owns the screen; nothing may stack over it.
  const connected = useConnectionStatus().data?.state === "connected";
  const shown = open && connected;
  const archivedShown = archivedOpen && connected;

  useEffect(() => {
    if (connected) return;
    if (open) closeSettingsPopover();
    if (archivedOpen) closeArchivedWorktrees();
  }, [open, archivedOpen, connected]);

  return (
    <>
      <button
        ref={anchorRef}
        type="button"
        onClick={toggleSettingsPopover}
        aria-label="Settings"
        title="Settings"
        aria-haspopup="dialog"
        aria-expanded={shown || archivedShown}
        className={cx(
          "inline-flex h-[26px] w-[26px] shrink-0 items-center justify-center rounded-control text-ink-dim",
          "hover:bg-edge/60 hover:text-ink focus:outline-none focus-visible:bg-edge/60 focus-visible:text-ink",
          (shown || archivedShown) && "bg-accent-soft text-ink hover:bg-accent-soft",
        )}
      >
        <Icon name="settings" />
      </button>
      <SettingsPopover open={shown} onClose={closeSettingsPopover} anchorRef={anchorRef} />
      <ArchivedWorktreesPopover
        open={archivedShown}
        onClose={closeArchivedWorktrees}
        anchorRef={anchorRef}
      />
    </>
  );
}
