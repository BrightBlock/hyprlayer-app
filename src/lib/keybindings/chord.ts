/**
 * Chord normalization, parsing, and per-OS display — the OS-agnostic core every
 * other keybinding module builds on. No React, no DOM beyond the `KeyboardEvent`
 * shape, so it's exhaustively unit-testable.
 *
 * A `Chord` is a canonical string: lowercase, modifiers first in a fixed order,
 * then the key token, joined by `+` (e.g. `"mod+shift+k"`). `mod` is the
 * abstract primary modifier — ⌘ on macOS, Ctrl elsewhere — so a single catalog
 * default renders and matches correctly on both platforms.
 */

/** A normalized chord, e.g. "mod+shift+k". `mod` = ⌘ on macOS, Ctrl elsewhere. */
export type Chord = string;

/** Canonical modifier order. `mod` is the primary (⌘/Ctrl); `meta`/`ctrl` are
 *  the literal secondary keys (the Windows key, or Control on macOS). */
const MOD_ORDER = ["mod", "meta", "ctrl", "alt", "shift"] as const;
const MOD_SET = new Set<string>(MOD_ORDER);

/** Keys that are *only* a modifier — pressing them alone never makes a chord. */
const BARE_MODIFIER_KEYS = new Set([
  "Shift",
  "Control",
  "Alt",
  "Meta",
  "CapsLock",
  "AltGraph",
  "Fn",
  "FnLock",
  "Hyper",
  "Super",
]);

/** Punctuation / whitespace `e.key` characters → stable token names. */
const PUNCT_TOKENS: Record<string, string> = {
  ",": "comma",
  ".": "period",
  "/": "slash",
  "\\": "backslash",
  "[": "bracketleft",
  "]": "bracketright",
  ";": "semicolon",
  "'": "quote",
  "`": "backquote",
  "-": "minus",
  "=": "equal",
  " ": "space",
};

/** Named multi-character `e.key` values → stable lowercase tokens. (Space is a
 *  single char, so it lives in PUNCT_TOKENS, not here.) */
const NAMED_TOKENS: Record<string, string> = {
  Enter: "enter",
  Escape: "escape",
  Tab: "tab",
  Backspace: "backspace",
  Delete: "delete",
  ArrowUp: "up",
  ArrowDown: "down",
  ArrowLeft: "left",
  ArrowRight: "right",
};

/** Map a raw `e.key` to its canonical token, or null if it's unusable. */
function keyToken(key: string): string | null {
  if (!key) return null;
  if (NAMED_TOKENS[key]) return NAMED_TOKENS[key];
  if (key.length === 1) {
    if (PUNCT_TOKENS[key]) return PUNCT_TOKENS[key];
    // Letters lowercase (so Shift is carried as a modifier, not the case);
    // digits and any other single char pass through lowercased.
    return key.toLowerCase();
  }
  // Unknown named key (F-keys, media keys, …): use a lowercased token so it's
  // at least stable and round-trippable, even if we don't pretty-print it.
  return key.toLowerCase();
}

/** Order a set of modifier tokens canonically. */
function sortMods(mods: Iterable<string>): string[] {
  const present = new Set(mods);
  return MOD_ORDER.filter((m) => present.has(m));
}

/**
 * Normalize a keydown into a canonical Chord, or null if it's a bare modifier
 * or an unusable key. On macOS the Cmd key maps to `mod` (Control stays the
 * literal `ctrl`); elsewhere Ctrl maps to `mod` (the Windows/Super key stays the
 * literal `meta`).
 */
export function eventToChord(
  e: Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey">,
  isMac: boolean,
): Chord | null {
  if (BARE_MODIFIER_KEYS.has(e.key)) return null;
  const token = keyToken(e.key);
  if (!token) return null;

  // Each entry is [pressed, token] in canonical MOD_ORDER, so filtering the
  // pressed ones needs no re-sort. The non-primary of the Cmd/Ctrl pair is kept
  // as its literal token, so a recorded chord that genuinely uses it (rare)
  // still round-trips.
  const modifiers: Array<[boolean, string]> = [
    [isMac ? e.metaKey : e.ctrlKey, "mod"],
    [isMac ? e.ctrlKey : e.metaKey, isMac ? "ctrl" : "meta"],
    [e.altKey, "alt"],
    [e.shiftKey, "shift"],
  ];

  return [...modifiers.filter(([pressed]) => pressed).map(([, mod]) => mod), token].join("+");
}

/** Split a chord into its canonically-ordered modifiers and trailing key. */
function parseChord(raw: string): { mods: string[]; key: string } {
  const parts = raw
    .toLowerCase()
    .split("+")
    .map((p) => p.trim())
    .filter(Boolean);
  const mods = sortMods(parts.filter((p) => MOD_SET.has(p)));
  const keys = parts.filter((p) => !MOD_SET.has(p));
  // The last non-modifier token is the key (defends against odd input).
  return { mods, key: keys[keys.length - 1] ?? "" };
}

/** Canonicalize a hand-written / default chord (sort modifiers, lowercase). */
export function normalizeChord(raw: string): Chord {
  const { mods, key } = parseChord(raw);
  return [...mods, key].filter(Boolean).join("+");
}

/** True when a chord includes a primary/secondary modifier (not just Shift). */
export function hasPrimaryModifier(chord: Chord): boolean {
  return chord.split("+").some((p) => MOD_SET.has(p) && p !== "shift");
}

/** macOS modifier glyphs, in canonical order. */
const MAC_MOD_GLYPH: Record<string, string> = {
  mod: "⌘",
  meta: "⌘",
  ctrl: "⌃",
  alt: "⌥",
  shift: "⇧",
};

/** Windows/Linux modifier labels, in canonical order. */
const OTHER_MOD_LABEL: Record<string, string> = {
  mod: "Ctrl",
  meta: "Win",
  ctrl: "Ctrl",
  alt: "Alt",
  shift: "Shift",
};

/** Key token → macOS display glyph (special keys), else the uppercased token. */
const MAC_KEY_GLYPH: Record<string, string> = {
  enter: "↩",
  escape: "⎋",
  tab: "⇥",
  backspace: "⌫",
  delete: "⌦",
  space: "Space",
  up: "↑",
  down: "↓",
  left: "←",
  right: "→",
};

/** Key token → Windows/Linux display label (special keys), else uppercased. */
const OTHER_KEY_LABEL: Record<string, string> = {
  enter: "Enter",
  escape: "Esc",
  tab: "Tab",
  backspace: "Backspace",
  delete: "Delete",
  space: "Space",
  up: "Up",
  down: "Down",
  left: "Left",
  right: "Right",
};

/** Symbols shared across platforms for punctuation tokens. */
const PUNCT_SYMBOL: Record<string, string> = {
  comma: ",",
  period: ".",
  slash: "/",
  backslash: "\\",
  bracketleft: "[",
  bracketright: "]",
  semicolon: ";",
  quote: "'",
  backquote: "`",
  minus: "-",
  equal: "=",
};

function keyDisplay(token: string, isMac: boolean): string {
  const special = isMac ? MAC_KEY_GLYPH[token] : OTHER_KEY_LABEL[token];
  if (special) return special;
  if (PUNCT_SYMBOL[token]) return PUNCT_SYMBOL[token];
  // Letters/digits and any other bare token: uppercase reads best in both styles.
  return token.toUpperCase();
}

/**
 * Pretty display for the host OS: "⌘⇧K" on macOS (concatenated glyphs),
 * "Ctrl+Shift+K" on Windows/Linux. Returns "" for an empty chord.
 */
export function formatChord(chord: Chord, isMac: boolean): string {
  if (!chord) return "";
  const { mods, key } = parseChord(chord);
  const keyStr = key ? keyDisplay(key, isMac) : "";

  if (isMac) {
    const glyphs = mods.map((m) => MAC_MOD_GLYPH[m] ?? "").join("");
    return `${glyphs}${keyStr}`;
  }
  const labels = mods.map((m) => OTHER_MOD_LABEL[m] ?? "");
  return [...labels, keyStr].filter(Boolean).join("+");
}
