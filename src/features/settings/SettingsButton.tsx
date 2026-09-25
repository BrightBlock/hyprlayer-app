import { useEffect, useRef } from "react";

import { useConnectionStatus } from "@/lib/queries";
import { Icon } from "@/components/Icon";
import { cx } from "@/components/ui";

import { SettingsPopover } from "./SettingsPopover";
import {
  closeSettingsPopover,
  toggleSettingsPopover,
  useSettingsPopover,
} from "./settingsPopoverStore";

/** The status bar's gear; owns the Settings popover. */
export function SettingsButton() {
  const anchorRef = useRef<HTMLButtonElement>(null);
  const open = useSettingsPopover((s) => s.open);
  // While disconnected the blocking HostsDialog owns the screen; nothing may stack over it.
  const connected = useConnectionStatus().data?.state === "connected";
  const shown = open && connected;

  useEffect(() => {
    if (open && !connected) closeSettingsPopover();
  }, [open, connected]);

  return (
    <>
      <button
        ref={anchorRef}
        type="button"
        onClick={toggleSettingsPopover}
        aria-label="Settings"
        title="Settings"
        aria-haspopup="dialog"
        aria-expanded={shown}
        className={cx(
          "flex h-6 w-[26px] shrink-0 items-center justify-center rounded-control text-ink-dim",
          "hover:bg-edge/60 hover:text-ink focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/60",
          shown && "bg-edge/60 text-ink",
        )}
      >
        <Icon name="settings" size="xs" />
      </button>
      <SettingsPopover open={shown} onClose={closeSettingsPopover} anchorRef={anchorRef} />
    </>
  );
}
