
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
      if (e.defaultPrevented) return;
      if (e.repeat) return;
      const chord = eventToChord(e, IS_MACOS);
      if (!chord) return;
      if (commandIndex.has(chord)) return;
      if (!hasPrimaryModifier(chord) && isEditableTarget(e.target)) return;
      const prompt = prompts.find((p) => p.chord && normalizeChord(p.chord) === chord);
      if (!prompt || !activeWorkstream) return;
      if (!isRunnablePrompt(prompt, activeWorkstream.item.projectPath)) return;
      e.preventDefault();
      void runCustomPrompt(activeWorkstream, prompt);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [prompts, commandIndex, activeWorkstream]);
}
