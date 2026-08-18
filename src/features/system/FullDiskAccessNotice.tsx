import { useEffect, useRef } from "react";

import { commands } from "@/lib/bindings";
import { useBootstrap, useFullDiskAccess, useSetAppSettings } from "@/lib/queries";
import { useToast } from "@/lib/toast";

/**
 * Raises a sticky warning toast when the app lacks Full Disk Access.
 *
 * macOS has no prompt for this grant, a process without it just gets EPERM,
 * so its absence is silent. What the user sees instead is the promptable
 * "access data from other apps" dialog, fired whenever a CLI in one of our PTYs
 * reaches another app's container. Full Disk Access supersedes that check. A
 * stale grant reads the same as none (System Settings shows it on while every
 * gated read fails), so the copy says to remove and re-add.
 *
 * Renders nothing: it drives the shared toast. The toast is sticky (it carries
 * an action) so it won't time out, and dismissing it persists so a later launch
 * doesn't re-nag.
 */
export function FullDiskAccessNotice() {
  const { data: access } = useFullDiskAccess();
  const bootstrap = useBootstrap();
  const setSettings = useSetAppSettings();
  const fired = useRef(false);

  const settings = bootstrap.data?.settings;

  // `setAppSettings` is a whole-object replace, so the dismissal must merge into
  // the *freshest* settings, not the snapshot captured when the toast fired,
  // otherwise dismissing reverts anything the user changed while it was up.
  const settingsRef = useRef(settings);
  useEffect(() => {
    settingsRef.current = settings;
  }, [settings]);

  // Once access is confirmed, clear any past dismissal so a later regression
  // (an ad-hoc rebuild invalidating the grant, the case this feature targets)
  // surfaces the warning again instead of staying silent forever.
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
                console.error("open Full Disk Access settings:", res.error);
              }
            } catch (e) {
              console.error("open Full Disk Access settings:", e);
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
