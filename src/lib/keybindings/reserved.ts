
import { normalizeChord, type Chord } from "./chord";

const MAC_RESERVED = [
  "mod+q",
  "mod+h",
  "mod+m",
  "mod+space",
  "mod+tab",
  "mod+shift+q",
  "mod+shift+3",
  "mod+shift+4",
];

const OTHER_RESERVED = [
  "mod+tab",
  "alt+f4",
  "alt+tab",
  "ctrl+alt+delete",
  "mod+shift+escape",
];

function buildSet(chords: string[]): Set<Chord> {
  return new Set(chords.map(normalizeChord));
}

const RESERVED: Record<"mac" | "other", Set<Chord>> = {
  mac: buildSet(MAC_RESERVED),
  other: buildSet(OTHER_RESERVED),
};

export function isReserved(chord: Chord, isMac: boolean): boolean {
  if (!chord) return false;
  return RESERVED[isMac ? "mac" : "other"].has(normalizeChord(chord));
}
