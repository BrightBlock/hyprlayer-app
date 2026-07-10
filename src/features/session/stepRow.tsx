/**
 * The transcript's collapsible-row shell, shared by every disclosure in the
 * conversation: thinking, tool, tool group, subagent, skill, and the prompt card.
 */

import { ChevronRightIcon } from "@/components/icons";

const STEP_ROW_BASE =
  "group -ml-1.5 flex w-[calc(100%+0.375rem)] items-center gap-2 rounded px-1.5 py-1 text-left";

/** A working-step row on the transcript's own background. */
export const STEP_ROW = `${STEP_ROW_BASE} hover:bg-panel-2`;

/** The same row on a `bg-panel-2` surface (the user bubble), where [`STEP_ROW`]'s
 *  hover tint would be the surface color and so read as no hover at all. */
export const STEP_ROW_ON_PANEL = `${STEP_ROW_BASE} hover:bg-edge/40`;

/** The disclosure chevron for a step row. `reveal="hover"` keeps it hidden until
 *  the row is hovered or open (thinking/tool); `"always"` shows it (group/subagent). */
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
