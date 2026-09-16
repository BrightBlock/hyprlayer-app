import { useEffect } from "react";
import { relaunch } from "@tauri-apps/plugin-process";

import { events } from "@/lib/bindings";
import { useToast } from "@/lib/toast";

// Nothing respawns the server (ADR-0009); relaunching the app takes the
// supervisor's connect-or-spawn path. A toast with an action stays until acted
// on or dismissed.
export function ServerDisconnectedNotice() {
  useEffect(() => {
    const unlisten = events.serverDisconnected.listen(() => {
      useToast.getState().show(
        "Lost connection to the Hyprlayer server",
        "Sessions and terminals keep running on it. Relaunch to reconnect.",
        {
          tone: "warn",
          action: { label: "Relaunch", busyLabel: "Relaunching…", onClick: () => relaunch() },
        },
      );
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);

  return null;
}
