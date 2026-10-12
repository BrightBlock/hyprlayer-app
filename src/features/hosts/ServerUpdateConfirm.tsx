import { useEffect } from "react";

import { connectionName } from "@/lib/domain/hosts";
import { errorMessage } from "@/lib/ipc";
import { useConnectionStatus } from "@/lib/queries";
import { useToast } from "@/lib/toast";
import { ConfirmDialog } from "@/components/ui";
import { closeServerUpdate, useHostsUiStore } from "./hostsUi";
import { updateServer } from "./serverAutoUpdate";

/** Confirms the status bar's server update, which then runs in the bar rather than a dialog. */
export function ServerUpdateConfirm() {
  const open = useHostsUiStore((s) => s.update);
  const connection = useConnectionStatus().data;
  const outdated = connection?.state === "connected" && connection.serverOutdated;
  const where = connectionName(connection) ?? "this machine";

  // Closed, not hidden: a later update would otherwise reopen it unasked.
  useEffect(() => {
    if (open && !outdated) closeServerUpdate();
  }, [open, outdated]);

  function confirm() {
    closeServerUpdate();
    if (!connection) return;
    updateServer(connection.host, connection.serverRelease).catch((e: unknown) => {
      useToast.getState().show("Server update failed", errorMessage(e), { tone: "bad" });
    });
  }

  return (
    <ConfirmDialog
      open={open && outdated}
      onClose={closeServerUpdate}
      title="Update the server?"
      confirmLabel="Update server"
      confirmVariant="danger"
      onConfirm={confirm}
    >
      <p className="text-body leading-prose text-ink-dim">
        Every agent and terminal running on <span className="text-ink">{where}</span> will stop.
        Hyprlayer reconnects once the updated server is up.
      </p>
    </ConfirmDialog>
  );
}
