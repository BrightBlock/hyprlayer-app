import { useEffect, useState } from "react";

import { commands, events } from "@/lib/bindings";
import { Button, Dialog, ErrorNote } from "@/components/ui";
import { IS_MACOS } from "@/lib/platform";

/**
 * "Quit Hyprlayer?" — raised by the backend on every user-initiated quit (⌘Q,
 * the window's close button) while Settings → General → "Confirm before
 * quitting" is on.
 *
 * The backend has already cancelled that exit before the event lands, so
 * dismissing this just resumes the app; only `quitApp` actually exits. Checking
 * "Don't ask again" rides along with the confirmation — the same command
 * persists the preference before exiting, so the write can't lose a race with
 * the process going away.
 */
export function ConfirmQuitDialog() {
  const [open, setOpen] = useState(false);
  const [dontAskAgain, setDontAskAgain] = useState(false);
  const [quitting, setQuitting] = useState(false);
  const [error, setError] = useState<unknown>(null);

  useEffect(() => {
    const unlisten = events.quitRequested.listen(() => {
      // Every prompt starts fresh: an unchecked box, no stale error.
      setDontAskAgain(false);
      setError(null);
      setQuitting(false);
      setOpen(true);
    });
    return () => {
      void unlisten.then((off) => off());
    };
  }, []);

  const close = () => {
    if (quitting) return;
    setOpen(false);
  };

  const quit = () => {
    if (quitting) return;
    setQuitting(true);
    setError(null);
    void commands.quitApp(dontAskAgain).then(
      (res) => {
        // Only reachable if the exit didn't happen — e.g. persisting
        // "Don't ask again" failed, which aborts before `app.exit`.
        if (res.status === "error") {
          setError(res.error);
          setQuitting(false);
        }
      },
      (e: unknown) => {
        setError(e);
        setQuitting(false);
      },
    );
  };

  return (
    <Dialog open={open} onClose={close} title="Quit Hyprlayer?">
      <div className="space-y-3">
        <p className="text-sm leading-relaxed text-ink-dim">
          Running agents, terminals, and lifecycle scripts all stop when the app quits.
        </p>

        {error != null && <ErrorNote error={error} />}

        <div className="flex items-center justify-between gap-4">
          {/* Bottom-left, deliberately quiet: the opt-out rides along with the
              confirmation rather than competing with it. */}
          <label
            className="flex cursor-pointer items-center gap-1.5 text-xs text-ink-dim hover:text-ink"
            title={`${IS_MACOS ? "⌘Q" : "Closing the window"} quits immediately. Re-enable in Settings → General.`}
          >
            <input
              type="checkbox"
              className="h-3 w-3 shrink-0 cursor-pointer accent-accent"
              checked={dontAskAgain}
              onChange={(e) => setDontAskAgain(e.target.checked)}
              disabled={quitting}
            />
            Don't ask again
          </label>

          <div className="flex shrink-0 gap-2">
            <Button type="button" onClick={close} disabled={quitting}>
              Cancel
            </Button>
            <Button type="button" variant="danger" busy={quitting} autoFocus onClick={quit}>
              Quit
            </Button>
          </div>
        </div>
      </div>
    </Dialog>
  );
}
