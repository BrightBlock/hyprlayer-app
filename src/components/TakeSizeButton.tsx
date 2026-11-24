import { commands } from "@/lib/bindings";
import { Icon } from "./Icon";
import { Button } from "./ui";

/** `fit` is this view's own size, sent first when the view hasn't been sizing the pty. */
export function TakeSizeButton({
  terminalId,
  fit,
}: {
  terminalId: () => string | undefined;
  fit?: () => { cols: number; rows: number } | undefined;
}) {
  return (
    <Button
      size="sm"
      className="absolute right-3 top-3 z-10 shadow-pop"
      onClick={() => {
        const id = terminalId();
        if (!id) return;
        const grid = fit?.();
        // Requests run on their own tasks, so the fit lands before the take that applies it.
        void (async () => {
          if (grid && grid.cols > 0 && grid.rows > 0) await commands.terminalResize(id, grid.cols, grid.rows);
          await commands.terminalTakeSize(id);
        })();
      }}
    >
      <Icon name="takeSize" size="xs" />
      Use this window's size
    </Button>
  );
}
