
import { normalizeChord, type Chord } from "./chord";
import { alternateChords, COMMANDS, commandById, defaultChord, type CommandDef } from "./commands";

export type Overrides = Record<string, string | null>;

export function hasOverride(overrides: Overrides, id: string): boolean {
  return Object.prototype.hasOwnProperty.call(overrides, id);
}

function resolveChord(overrides: Overrides, cmd: CommandDef, isMac: boolean): Chord | null {
  if (!hasOverride(overrides, cmd.id)) return defaultChord(cmd, isMac);
  const override = overrides[cmd.id];
  return override == null ? null : normalizeChord(override);
}

export function effectiveChords(overrides: Overrides, isMac: boolean): Map<string, Chord | null> {
  return new Map(COMMANDS.map((cmd) => [cmd.id, resolveChord(overrides, cmd, isMac)]));
}

function resolveAlternates(overrides: Overrides, cmd: CommandDef, isMac: boolean): Chord[] {
  return hasOverride(overrides, cmd.id) ? [] : alternateChords(cmd, isMac);
}

export function effectiveAlternates(overrides: Overrides, id: string, isMac: boolean): Chord[] {
  const cmd = commandById(id);
  return cmd ? resolveAlternates(overrides, cmd, isMac) : [];
}

export function effectiveChord(overrides: Overrides, id: string, isMac: boolean): Chord | null {
  const cmd = commandById(id);
  return cmd ? resolveChord(overrides, cmd, isMac) : null;
}

export function chordToCommand(overrides: Overrides, isMac: boolean): Map<Chord, string> {
  const index = new Map<Chord, string>();
  for (const [id, chord] of effectiveChords(overrides, isMac)) if (chord) index.set(chord, id);
  for (const cmd of COMMANDS)
    for (const alt of resolveAlternates(overrides, cmd, isMac))
      if (!index.has(alt)) index.set(alt, cmd.id);
  return index;
}

export function conflictsFor(
  chord: Chord,
  selfId: string,
  overrides: Overrides,
  isMac: boolean,
): string[] {
  const norm = normalizeChord(chord);
  if (!norm) return [];
  return COMMANDS.filter(
    (cmd) =>
      cmd.id !== selfId &&
      (resolveChord(overrides, cmd, isMac) === norm ||
        resolveAlternates(overrides, cmd, isMac).includes(norm)),
  ).map((cmd) => cmd.id);
}
