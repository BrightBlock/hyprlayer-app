import { useEffect, useRef } from "react";
import { History } from "lucide-react";

import { useCommand } from "@/lib/keybindings/registry";
import { useConnectionStatus } from "@/lib/queries";
import { ICON } from "@/components/lucide";
import { cx } from "@/components/ui";

import { ArchivedAgentsPopover } from "./ArchivedAgentsPopover";
import { closeArchivedAgents, toggleArchivedAgents, useArchivedAgents } from "./archivedAgentsStore";
import { useWorkstreamTabsStore } from "./workstreamTabs";

/** The status bar's archive button; owns the workstream's Archived agents popover. */
export function ArchivedAgentsButton({ workstreamId }: { workstreamId: string }) {
  const anchorRef = useRef<HTMLButtonElement>(null);
  const open = useArchivedAgents((s) => s.open);
  const count = useWorkstreamTabsStore((s) => s.byWs[workstreamId]?.archived?.length ?? 0);
  const status = useConnectionStatus().data;
  // While disconnected the blocking HostsDialog owns the screen; nothing may stack over it.
  const disconnected = status !== undefined && status.state !== "connected";
  const shown = open && !disconnected;

  useCommand("view.toggleArchivedAgents", toggleArchivedAgents);
  useEffect(() => {
    if (open && disconnected) closeArchivedAgents();
  }, [open, disconnected]);
  // The list is one workstream's; switching or leaving closes it.
  useEffect(() => closeArchivedAgents, [workstreamId]);

  return (
    <>
      <button
        ref={anchorRef}
        type="button"
        onClick={toggleArchivedAgents}
        aria-label="Archived agents"
        title="Archived agents"
        aria-haspopup="dialog"
        aria-expanded={shown}
        className={cx(
          "flex h-6 min-w-[26px] shrink-0 items-center justify-center gap-1 rounded-control px-1 text-ink-dim",
          "hover:bg-edge/60 hover:text-ink focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/60",
          shown && "bg-edge/60 text-ink",
        )}
      >
        <History {...ICON} size={14} className="shrink-0" />
        {count > 0 && <span className="tabular-nums">{count}</span>}
      </button>
      <ArchivedAgentsPopover
        workstreamId={workstreamId}
        open={shown}
        onClose={closeArchivedAgents}
        anchorRef={anchorRef}
      />
    </>
  );
}
