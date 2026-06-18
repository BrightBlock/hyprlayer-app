/**
 * Resolver: merge the catalog defaults with the user's overrides into the
 * effective binding for each command on the host OS, build the reverse index the
 * dispatcher uses, and detect conflicts for the Settings UI. Pure functions over
 * the catalog + an overrides map.
 */

import { normalizeChord, type Chord } from "./chord";
import { COMMANDS, defaultChord } from "./commands";

/** command id → chord (custom binding), or null (explicitly unbound). An absent
 *  key means "use the catalog default". Mirrors the backend `keybindings.overrides`. */
export type Overrides = Record<string, string | null>;

/** Whether a command id has an explicit override (custom chord or unbound) —
 *  vs. tracking the catalog default (absent key). */
export function hasOverride(overrides: Overrides, id: string): boolean {
  return Object.prototype.hasOwnProperty.call(overrides, id);
}

/**
 * Effective chord per command for this OS: an override (including `null` =
 * unbound) wins; otherwise the catalog default. Every command in the catalog is
 * present in the returned map.
 */
export function effectiveChords(overrides: Overrides, isMac: boolean): Map<string, Chord | null> {
  const map = new Map<string, Chord | null>();
  for (const cmd of COMMANDS) {
    if (hasOverride(overrides, cmd.id)) {
      const o = overrides[cmd.id];
      map.set(cmd.id, o == null ? null : normalizeChord(o));
    } else {
      map.set(cmd.id, defaultChord(cmd, isMac));
    }
  }
  return map;
}

/**
 * Reverse index chord → command id, for the dispatcher. Unbound commands are
 * excluded. On a collision the later command in catalog order wins — a conflict
 * the Settings UI should already have flagged via `conflictsFor`.
 */
export function chordToCommand(overrides: Overrides, isMac: boolean): Map<Chord, string> {
  const map = new Map<Chord, string>();
  for (const [id, chord] of effectiveChords(overrides, isMac)) {
    if (chord) map.set(chord, id);
  }
  return map;
}

/** Command ids (other than `selfId`) whose effective binding equals `chord`. */
export function conflictsFor(
  chord: Chord,
  selfId: string,
  overrides: Overrides,
  isMac: boolean,
): string[] {
  const norm = normalizeChord(chord);
  if (!norm) return [];
  const out: string[] = [];
  for (const [id, c] of effectiveChords(overrides, isMac)) {
    if (id !== selfId && c && c === norm) out.push(id);
  }
  return out;
}
