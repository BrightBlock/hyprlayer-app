import { useEffect, useRef } from "react";

import { useConnectionStatus } from "@/lib/queries";
import { Icon } from "@/components/Icon";
import { cx } from "@/components/ui";

import { AgentSessionsPopover } from "./AgentSessionsPopover";
import { closeAgentSessions, toggleAgentSessions, useAgentSessions } from "./agentSessionsStore";

/** The status bar's sessions button, flush left of Server; owns the workstream's Agent sessions popover. */
export function AgentSessionsButton({ workstreamId }: { workstreamId: string }) {
  const anchorRef = useRef<HTMLButtonElement>(null);
  const open = useAgentSessions((s) => s.open);
  const status = useConnectionStatus().data;
  // While disconnected the blocking HostsDialog owns the screen; nothing may stack over it.
  const disconnected = status !== undefined && status.state !== "connected";
  const shown = open && !disconnected;

  useEffect(() => {
    if (open && disconnected) closeAgentSessions();
  }, [open, disconnected]);
  // The list is one workstream's; switching or leaving closes it.
  useEffect(() => closeAgentSessions, [workstreamId]);

  return (
    <>
      <button
        ref={anchorRef}
        type="button"
        onClick={toggleAgentSessions}
        aria-label="Agent sessions"
        title="Agent sessions"
        aria-haspopup="dialog"
        aria-expanded={shown}
        className={cx(
          "flex h-6 min-w-[26px] shrink-0 items-center justify-center rounded-control px-1 text-ink-dim",
          "hover:bg-edge/60 hover:text-ink focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/60",
          shown && "bg-edge/60 text-ink",
        )}
      >
        <Icon name="sessions" size="xs" />
      </button>
      <AgentSessionsPopover
        workstreamId={workstreamId}
        open={shown}
        onClose={closeAgentSessions}
        anchorRef={anchorRef}
      />
    </>
  );
}
