import { useEffect, useRef } from "react";

import { commands } from "@/lib/bindings";
import { log } from "@/lib/log";
import { useBootstrap, useFullDiskAccess, useSetAppSettings } from "@/lib/queries";
import { useToast } from "@/lib/toast";

export function FullDiskAccessNotice() {
  const { data: access } = useFullDiskAccess();
  const bootstrap = useBootstrap();
  const setSettings = useSetAppSettings();
  const fired = useRef(false);

  const settings = bootstrap.data?.settings;

  const settingsRef = useRef(settings);
  useEffect(() => {
    settingsRef.current = settings;
  }, [settings]);

  useEffect(() => {
    if (access === "granted" && settings?.fullDiskAccessNoticeDismissed) {
      setSettings.mutate({ ...settings, fullDiskAccessNoticeDismissed: false });
    }
  }, [access, settings, setSettings]);

  useEffect(() => {
    if (fired.current) return;
    if (access !== "denied" || !settings || settings.fullDiskAccessNoticeDismissed) return;
    fired.current = true;
    useToast.getState().show(
      "Full Disk Access needed",
      "Add Hyprlayer in Settings and restart to stop the repeated “access data from other apps” prompts. If it looks enabled, remove and re-add it.",
      {
        tone: "warn",
        action: {
          label: "Open Settings…",
          onClick: async () => {
            try {
              const res = await commands.openFullDiskAccessSettings();
              if (res.status === "error") {
                log.error("full-disk-access", "open System Settings failed", res.error);
              }
            } catch (e) {
              log.error("full-disk-access", "open System Settings failed", e);
            }
          },
        },
        onDismiss: () => {
          const s = settingsRef.current;
          if (s) setSettings.mutate({ ...s, fullDiskAccessNoticeDismissed: true });
        },
      },
    );
  }, [access, settings, setSettings]);

  return null;
}
