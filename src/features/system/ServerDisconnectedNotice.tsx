import { useEffect, useRef } from "react";

import { events } from "@/lib/bindings";
import { errorMessage, errorRemedy } from "@/lib/ipc";
import { useConnectionStatus } from "@/lib/queries";
import { useToast } from "@/lib/toast";
import { isPlannedRestart } from "@/features/hosts/plannedRestart";
import { useReconnectStore } from "@/features/hosts/reconnectState";
import { useServerAutoUpdate } from "@/features/hosts/serverAutoUpdate";

// The reconnect loop (features/hosts/reconnect) dials again on every drop, so
// there is nothing for the user to do here.
const TITLE = "Lost connection to the Hyprlayer server";
const MESSAGE = "Sessions and terminals keep running on it. Reconnecting…";

function showLost() {
  useToast.getState().show(TITLE, MESSAGE, { tone: "warn" });
}

export function ServerDisconnectedNotice() {
  const status = useConnectionStatus();
  const connected = status.data?.state === "connected";
  const lost = status.data?.state === "lost";
  const { halted, keyChanged } = useReconnectStore();
  const updating = useServerAutoUpdate((s) => s.updating);
  const wasUpdating = useRef(updating);
  const keyToast = useRef<string | null>(null);
  const keyTitle = keyChanged ? errorMessage(keyChanged) : null;
  const keyRemedy = keyChanged ? errorRemedy(keyChanged) : null;

  // Keyed on every refetch, not just the flag: a reconnect quick enough that no
  // refetch saw the drop still has to take the toast down.
  useEffect(() => {
    const toast = useToast.getState();
    if (connected && toast.visible && (toast.title === TITLE || toast.title === keyToast.current)) {
      toast.hide();
    }
  }, [connected, status.dataUpdatedAt]);

  useEffect(() => {
    const unlisten = events.serverDisconnected.listen(() => {
      if (isPlannedRestart()) return;
      showLost();
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);

  // An update's drop toasted nothing; a server that never came back is an outage.
  useEffect(() => {
    if (wasUpdating.current && !updating && lost && !halted && !keyChanged) showLost();
    wasUpdating.current = updating;
  }, [updating, lost, halted, keyChanged]);

  // Only the user can clear a changed key, and no dialog opens on its own to say so.
  useEffect(() => {
    if (!keyTitle) return;
    keyToast.current = keyTitle;
    useToast.getState().show(keyTitle, keyRemedy ?? "", { tone: "bad" });
  }, [keyTitle, keyRemedy]);

  return null;
}
