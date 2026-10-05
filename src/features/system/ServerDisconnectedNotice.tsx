import { useEffect } from "react";

import { events } from "@/lib/bindings";
import { useConnectionStatus } from "@/lib/queries";
import { useToast } from "@/lib/toast";
import { isPlannedRestart } from "@/features/hosts/plannedRestart";

// The reconnect loop (features/hosts/reconnect) dials again on every drop, so
// there is nothing for the user to do here.
const TITLE = "Lost connection to the Hyprlayer server";

export function ServerDisconnectedNotice() {
  const status = useConnectionStatus();
  const connected = status.data?.state === "connected";

  // Keyed on every refetch, not just the flag: a reconnect quick enough that no
  // refetch saw the drop still has to take the toast down.
  useEffect(() => {
    const toast = useToast.getState();
    if (connected && toast.visible && toast.title === TITLE) toast.hide();
  }, [connected, status.dataUpdatedAt]);

  useEffect(() => {
    const unlisten = events.serverDisconnected.listen(() => {
      if (isPlannedRestart()) return;
      useToast.getState().show(TITLE, "Sessions and terminals keep running on it. Reconnecting…", {
        tone: "warn",
      });
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);

  return null;
}
