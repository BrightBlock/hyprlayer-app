
import { ChevronRightIcon } from "@/components/icons";

const STEP_ROW_BASE =
  "group -ml-1.5 flex w-[calc(100%+0.375rem)] items-center gap-2 rounded px-1.5 py-1 text-left";

export const STEP_ROW = `${STEP_ROW_BASE} hover:bg-edge/50`;

export const STEP_ROW_ON_PANEL = STEP_ROW;

export const STEP_ROW_STATIC = STEP_ROW_BASE;

export function StepChevron({
  open,
  reveal = "always",
  size = 12,
}: {
  open: boolean;
  reveal?: "hover" | "always";
  size?: number;
}) {
  const visibility =
    reveal === "hover"
      ? open
        ? "rotate-90 text-ink-dim"
        : "text-ink-dim/0 group-hover:text-ink-dim"
      : `${open ? "rotate-90" : ""} text-ink-dim`;
  return (
    <ChevronRightIcon size={size} className={`ml-auto shrink-0 transition-all ${visibility}`} />
  );
}
