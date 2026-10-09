import { useEffect, useRef } from "react";

import { useCommand } from "@/lib/keybindings/registry";
import { useConnectionStatus, useWorkstreamSessions } from "@/lib/queries";
import { Icon } from "@/components/Icon";
import { cx } from "@/components/ui";

import { AgentSessionsPopover } from "./AgentSessionsPopover";
import {
  closeAgentSessions,
  toggleAgentSessions,
  toggleArchivedSessions,
  useAgentSessions,
} from "./agentSessionsStore";

/** The status bar's sessions button, flush left of Server; owns the workstream's current and archived sessions. */
export function AgentSessionsButton({ workstreamId }: { workstreamId: string }) {
  const anchorRef = useRef<HTMLButtonElement>(null);
  const open = useAgentSessions((s) => s.open);
  const current = (useWorkstreamSessions(workstreamId).data ?? []).filter((s) => s.archivedAt == null).length;
  const status = useConnectionStatus().data;
  // While disconnected the blocking HostsDialog owns the screen; nothing may stack over it.
  const disconnected = status !== undefined && status.state !== "connected";
  const shown = open && !disconnected;

  useCommand("view.toggleArchivedAgents", toggleArchivedSessions);
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
          "flex h-6 min-w-[26px] shrink-0 items-center justify-center gap-1 rounded-control px-1 text-ink-dim",
          "hover:bg-edge/60 hover:text-ink focus:outline-none focus-visible:bg-edge/60 focus-visible:text-ink",
          shown && "bg-edge/60 text-ink",
        )}
      >
        <Icon name="sessions" size="xs" />
        {current > 0 && <span className="tabular-nums">{current}</span>}
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
