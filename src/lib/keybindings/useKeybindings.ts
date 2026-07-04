/**
 * The single global keydown dispatcher. Mounted once (in `Root`): it builds the
 * chord → command index for the host OS and installs a `window` listener that
 * routes a pressed chord to the registered handler for its command.
 *
 * Guard rules:
 *  - While focus is in an editable element (input/textarea/contenteditable or a
 *    CodeMirror editor), a chord with NO primary modifier is ignored — so plain
 *    typing (`t`, `1`, …) is never hijacked, but `⌘1` still works mid-compose.
 *  - A chord only fires if a handler is currently registered (the context gate);
 *    otherwise the event passes through untouched.
 */

import { useEffect, useMemo } from "react";

import { IS_MACOS } from "../platform";
import { eventToChord, hasPrimaryModifier } from "./chord";
import { chordToCommand, type Overrides } from "./resolve";
import { ESCAPE_COMMAND_ID, runCommand } from "./registry";

/** Whether keyboard focus is in an editable element — the guard that keeps
 *  bare/shift-only chords from hijacking typing. Shared with the prompt-hotkey
 *  dispatcher (`usePromptHotkeys`), which must apply the exact same rule. */
export function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
  if (target.isContentEditable) return true;
  // CodeMirror (the doc editor + composer) — its content host isn't a textarea.
  if (target.closest(".cm-editor")) return true;
  return false;
}

export function useKeybindings(overrides: Overrides): void {
  const index = useMemo(() => chordToCommand(overrides, IS_MACOS), [overrides]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Bare Escape closes the topmost dismissible surface (no editable guard —
      // you must escape a dialog you're typing in); only consumed when handled,
      // so it still reaches the terminal/editor when nothing is open.
      if (e.key === "Escape" && !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) {
        if (runCommand(ESCAPE_COMMAND_ID)) e.preventDefault();
        return;
      }
      const chord = eventToChord(e, IS_MACOS);
      if (!chord) return;
      const id = index.get(chord);
      if (!id) return;
      // Never steal ordinary typing: a bare/shift-only chord is inert in editors.
      if (!hasPrimaryModifier(chord) && isEditableTarget(e.target)) return;
      // Context gate: only act when a handler is registered for this command.
      if (runCommand(id)) e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [index]);
}
