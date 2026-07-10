/**
 * System-reserved chords, per OS. Used for *warnings only* — recording one of
 * these surfaces an inline caution, but the user can still proceed (the OS will
 * usually win the keystroke anyway). Chords use the abstract `mod` modifier, so
 * `mod+w` means ⌘W on macOS and Ctrl+W elsewhere.
 *
 * Deliberately conservative: a combo the OS *always* swallows (Cmd+Q) belongs
 * here; one apps routinely rebind (Cmd+T) does not. `mod+comma` is explicitly
 * NOT reserved — it's the conventional Preferences/Settings shortcut. Nor is
 * `mod+w`: the macOS menu omits Close Window on purpose, so ⌘W reaches the
 * webview and belongs to Close Agent Tab (see src-tauri/src/lib.rs).
 */

import { normalizeChord, type Chord } from "./chord";

const MAC_RESERVED = [
  "mod+q", // Quit
  "mod+h", // Hide app
  "mod+m", // Minimize
  "mod+space", // Spotlight
  "mod+tab", // App switcher
  "mod+shift+q", // Log out
  "mod+shift+3", // Screenshot
  "mod+shift+4", // Screenshot region
];

const OTHER_RESERVED = [
  "mod+tab", // (often) tab switch
  "alt+f4", // Close window
  "alt+tab", // Window switcher
  "ctrl+alt+delete", // Secure attention
  "mod+shift+escape", // Task manager
];

function buildSet(chords: string[]): Set<Chord> {
  return new Set(chords.map(normalizeChord));
}

const RESERVED: Record<"mac" | "other", Set<Chord>> = {
  mac: buildSet(MAC_RESERVED),
  other: buildSet(OTHER_RESERVED),
};

/** True when `chord` collides with a known system-reserved combo on this OS. */
export function isReserved(chord: Chord, isMac: boolean): boolean {
  if (!chord) return false;
  return RESERVED[isMac ? "mac" : "other"].has(normalizeChord(chord));
}
