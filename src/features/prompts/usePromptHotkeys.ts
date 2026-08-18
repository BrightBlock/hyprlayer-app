/**
 * Global dispatcher for CUSTOM prompt shortcuts. Custom prompts aren't catalog
 * commands (they're user data, not stable ids), so they can't ride
 * `useKeybindings`; this hook installs a sibling `window` listener with the
 * same rules, bubble phase, editable-target guard, `preventDefault` only when
 * something ran, and defers to the catalog on any collision (a chord that maps
 * to an app command never fires a prompt). Built-in prompts need none of this:
 * Commit/Ship already dispatch through `ws.commit` / `ws.ship`.
 */

import { useEffect, useMemo } from "react";

import type { WorkstreamView } from "@/lib/bindings";
import { useBootstrap } from "@/lib/queries";
import { IS_MACOS } from "@/lib/platform";
import { eventToChord, hasPrimaryModifier, normalizeChord } from "@/lib/keybindings/chord";
import { chordToCommand, type Overrides } from "@/lib/keybindings/resolve";
import { isEditableTarget } from "@/lib/keybindings/useKeybindings";
import { customPrompts, isRunnablePrompt } from "./prompts";
import { runCustomPrompt } from "./runPrompt";

const EMPTY_OVERRIDES: Overrides = {};

export function usePromptHotkeys(activeWorkstream: WorkstreamView | null): void {
  const bootstrap = useBootstrap();
  const settings = bootstrap.data?.settings;
  const overrides = settings?.keybindings?.overrides ?? EMPTY_OVERRIDES;
  const prompts = useMemo(() => customPrompts(settings), [settings]);
  const commandIndex = useMemo(() => chordToCommand(overrides, IS_MACOS), [overrides]);

  useEffect(() => {
    if (prompts.every((p) => !p.chord)) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return; // an app command already took this chord
      if (e.repeat) return; // a held-down chord must not fire the prompt N times
      const chord = eventToChord(e, IS_MACOS);
      if (!chord) return;
      if (commandIndex.has(chord)) return; // catalog commands win collisions
      if (!hasPrimaryModifier(chord) && isEditableTarget(e.target)) return;
      const prompt = prompts.find((p) => p.chord && normalizeChord(p.chord) === chord);
      if (!prompt || !activeWorkstream) return;
      // An empty body for this project sends nothing, leave the chord to the
      // browser rather than swallowing it for a no-op run.
      if (!isRunnablePrompt(prompt, activeWorkstream.item.projectPath)) return;
      e.preventDefault();
      void runCustomPrompt(activeWorkstream, prompt);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [prompts, commandIndex, activeWorkstream]);
}
