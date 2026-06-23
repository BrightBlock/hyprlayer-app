/**
 * Resolver: merge the catalog defaults with the user's overrides into the
 * effective binding for each command on the host OS, build the reverse index the
 * dispatcher uses, and detect conflicts for the Settings UI. Pure functions over
 * the catalog + an overrides map.
 */

import { normalizeChord, type Chord } from "./chord";
import { COMMANDS, commandById, defaultChord, type CommandDef } from "./commands";

/** command id → chord (custom binding), or null (explicitly unbound). An absent
 *  key means "use the catalog default". Mirrors the backend `keybindings.overrides`. */
export type Overrides = Record<string, string | null>;

/** Whether a command id has an explicit override (custom chord or unbound) —
 *  vs. tracking the catalog default (absent key). */
export function hasOverride(overrides: Overrides, id: string): boolean {
  return Object.prototype.hasOwnProperty.call(overrides, id);
}

/** The effective chord for one command: an explicit override (a chord, or
 *  `null` = unbound) wins; otherwise the catalog default for this OS. */
function resolveChord(overrides: Overrides, cmd: CommandDef, isMac: boolean): Chord | null {
  if (!hasOverride(overrides, cmd.id)) return defaultChord(cmd, isMac);
  const override = overrides[cmd.id];
  return override == null ? null : normalizeChord(override);
}

/**
 * Effective chord per command for this OS: an override (including `null` =
 * unbound) wins; otherwise the catalog default. Every command in the catalog is
 * present in the returned map.
 */
export function effectiveChords(overrides: Overrides, isMac: boolean): Map<string, Chord | null> {
  return new Map(COMMANDS.map((cmd) => [cmd.id, resolveChord(overrides, cmd, isMac)]));
}

/** The effective chord for a single command id (an override wins, else the
 *  catalog default), or null if it's unbound or the id is unknown. For callers
 *  that match one specific chord without building the whole index — e.g. a chord
 *  the global dispatcher can't see because its editable-target guard swallows it. */
export function effectiveChord(overrides: Overrides, id: string, isMac: boolean): Chord | null {
  const cmd = commandById(id);
  return cmd ? resolveChord(overrides, cmd, isMac) : null;
}

/**
 * Reverse index chord → command id, for the dispatcher. Unbound commands are
 * excluded. On a collision the later command in catalog order wins — a conflict
 * the Settings UI should already have flagged via `conflictsFor`.
 */
export function chordToCommand(overrides: Overrides, isMac: boolean): Map<Chord, string> {
  return new Map(
    [...effectiveChords(overrides, isMac)].flatMap(([id, chord]) =>
      chord ? [[chord, id] as const] : [],
    ),
  );
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
  return [...effectiveChords(overrides, isMac)]
    .filter(([id, c]) => id !== selfId && c === norm)
    .map(([id]) => id);
}
