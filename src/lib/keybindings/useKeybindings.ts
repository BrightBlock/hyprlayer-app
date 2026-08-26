
import { useEffect, useMemo } from "react";

import { IS_MACOS } from "../platform";
import { eventToChord, hasPrimaryModifier } from "./chord";
import { chordToCommand, type Overrides } from "./resolve";
import { ESCAPE_COMMAND_ID, runCommand } from "./registry";

export function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
  if (target.isContentEditable) return true;
  if (target.closest(".cm-editor")) return true;
  return false;
}

export function useKeybindings(overrides: Overrides): void {
  const index = useMemo(() => chordToCommand(overrides, IS_MACOS), [overrides]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) {
        if (runCommand(ESCAPE_COMMAND_ID)) e.preventDefault();
        return;
      }
      const chord = eventToChord(e, IS_MACOS);
      if (!chord) return;
      const id = index.get(chord);
      if (!id) return;
      if (!hasPrimaryModifier(chord) && isEditableTarget(e.target)) return;
      if (runCommand(id)) e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [index]);
}
