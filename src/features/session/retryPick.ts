import type { Harness } from "@/lib/bindings";

import type { ShellTab } from "./workstreamTabs";

type ShellPicks = Pick<ShellTab, "model" | "effort" | "opencodeModel" | "codexModel" | "codexEffort">;

/** This tab's own pick for the harness; without one, a failed start's tab shows the row's model. */
function tabPick(harness: Harness, shell: ShellPicks): { model?: string; effort?: string } {
  switch (harness) {
    case "opencode":
      return { model: shell.opencodeModel };
    case "codex":
      return { model: shell.codexModel, effort: shell.codexEffort };
    default:
      return { model: shell.model, effort: shell.effort };
  }
}

/**
 * Resume writes only what the user picked on this tab; anything else leaves the row's own model and effort.
 * A pick of another harness than the row's moves the start there with everything the tab shows.
 */
export function retryWrites(
  harness: Harness,
  shell: ShellPicks,
  resolved: { model: string | null; effort: string | null },
  rowHarness?: Harness,
): { model: string | null; effort: string | null; harness?: Harness } {
  if (rowHarness && rowHarness !== harness) return { ...resolved, harness };
  const pick = tabPick(harness, shell);
  return {
    model: pick.model != null ? resolved.model : null,
    effort: pick.effort != null ? resolved.effort : null,
  };
}
