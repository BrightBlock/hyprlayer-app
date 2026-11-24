
import { Icon } from "@/components/Icon";

const STEP_ROW_BASE =
  "group -ml-1.5 flex w-[calc(100%+0.375rem)] items-center gap-2 rounded px-1.5 py-1 text-left";

export const STEP_ROW = `${STEP_ROW_BASE} hover:bg-edge/50`;

export const STEP_ROW_ON_PANEL = STEP_ROW;

export const STEP_ROW_STATIC = STEP_ROW_BASE;

export function StepChevron({
  open,
  reveal = "always",
}: {
  open: boolean;
  reveal?: "hover" | "always";
}) {
  const visibility =
    reveal === "hover"
      ? open
        ? "rotate-90 text-ink-dim"
        : "text-ink-dim opacity-0 group-hover:opacity-100"
      : `${open ? "rotate-90" : ""} text-ink-dim`;
  return (
    <Icon name="chevronRight" size="xs" className={`ml-auto transition-[opacity,rotate] ${visibility}`} />
  );
}
