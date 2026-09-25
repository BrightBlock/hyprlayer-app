import { useEffect, useState } from "react";

import { commands, events } from "@/lib/bindings";
import { ConfirmDialog } from "@/components/ui";
import { IS_MACOS } from "@/lib/platform";

export function ConfirmQuitDialog() {
  const [open, setOpen] = useState(false);
  const [dontAskAgain, setDontAskAgain] = useState(false);
  const [quitting, setQuitting] = useState(false);
  const [error, setError] = useState<unknown>(null);

  useEffect(() => {
    const unlisten = events.quitRequested.listen(() => {
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
    <ConfirmDialog
      open={open}
      onClose={close}
      title="Quit Hyprlayer?"
      confirmLabel="Quit"
      confirmVariant="danger"
      onConfirm={quit}
      busy={quitting}
      autoFocus="confirm"
      error={error}
      onDismissError={() => setError(null)}
    >
      <p className="text-body leading-prose text-ink-dim">
        Running agents, terminals, and lifecycle scripts all stop when the app quits.
      </p>

      <label
        className="flex cursor-pointer items-center gap-1.5 text-ui text-ink-dim hover:text-ink"
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
    </ConfirmDialog>
  );
}
