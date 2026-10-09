import { Icon } from "@/components/Icon";
import { cx } from "@/components/ui";
import { Dot } from "./parts";

export function ScriptsDockButton({
  open,
  running,
  onToggle,
}: {
  open: boolean;
  running: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-label="Terminal and scripts dock"
      aria-pressed={open}
      title={running ? "Scripts · a Run is active" : "Terminal & scripts dock"}
      className={cx(
        "relative flex h-6 w-[26px] shrink-0 items-center justify-center rounded-control text-ink-dim",
        "hover:bg-edge/60 hover:text-ink focus:outline-none focus-visible:bg-edge/60 focus-visible:text-ink",
        open && "bg-edge/60 text-ink",
      )}
    >
      <Icon name="console" size="sm" />
      {running && <Dot className="absolute right-0.5 top-0.5 bg-ship" />}
    </button>
  );
}
