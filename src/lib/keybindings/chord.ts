
export type Chord = string;

const MOD_ORDER = ["mod", "meta", "ctrl", "alt", "shift"] as const;
const MOD_SET = new Set<string>(MOD_ORDER);

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

const SHIFTED_TOKENS: Record<string, string> = {
  "{": "bracketleft",
  "}": "bracketright",
  ":": "semicolon",
  '"': "quote",
  "~": "backquote",
  _: "minus",
  "+": "equal",
  "|": "backslash",
  "<": "comma",
  ">": "period",
  "?": "slash",
  "!": "1",
  "@": "2",
  "#": "3",
  $: "4",
  "%": "5",
  "^": "6",
  "&": "7",
  "*": "8",
  "(": "9",
  ")": "0",
};

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
  PageUp: "pageup",
  PageDown: "pagedown",
};

function keyToken(key: string): string | null {
  if (!key) return null;
  if (NAMED_TOKENS[key]) return NAMED_TOKENS[key];
  if (key.length === 1) {
    if (SHIFTED_TOKENS[key]) return SHIFTED_TOKENS[key];
    if (PUNCT_TOKENS[key]) return PUNCT_TOKENS[key];
    return key.toLowerCase();
  }
  return key.toLowerCase();
}

function sortMods(mods: Iterable<string>): string[] {
  const present = new Set(mods);
  return MOD_ORDER.filter((m) => present.has(m));
}

export function eventToChord(
  e: Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey">,
  isMac: boolean,
): Chord | null {
  if (BARE_MODIFIER_KEYS.has(e.key)) return null;
  const token = keyToken(e.key);
  if (!token) return null;

  const modifiers: Array<[boolean, string]> = [
    [isMac ? e.metaKey : e.ctrlKey, "mod"],
    [isMac ? e.ctrlKey : e.metaKey, isMac ? "ctrl" : "meta"],
    [e.altKey, "alt"],
    [e.shiftKey, "shift"],
  ];

  return [...modifiers.filter(([pressed]) => pressed).map(([, mod]) => mod), token].join("+");
}

function parseChord(raw: string): { mods: string[]; key: string } {
  const parts = raw
    .toLowerCase()
    .split("+")
    .map((p) => p.trim())
    .filter(Boolean);
  const mods = sortMods(parts.filter((p) => MOD_SET.has(p)));
  const keys = parts.filter((p) => !MOD_SET.has(p));
  return { mods, key: keys[keys.length - 1] ?? "" };
}

export function normalizeChord(raw: string): Chord {
  const { mods, key } = parseChord(raw);
  return [...mods, key].filter(Boolean).join("+");
}

export function hasPrimaryModifier(chord: Chord): boolean {
  return chord.split("+").some((p) => MOD_SET.has(p) && p !== "shift");
}

const MAC_MOD_GLYPH: Record<string, string> = {
  mod: "⌘",
  meta: "⌘",
  ctrl: "⌃",
  alt: "⌥",
  shift: "⇧",
};

const OTHER_MOD_LABEL: Record<string, string> = {
  mod: "Ctrl",
  meta: "Win",
  ctrl: "Ctrl",
  alt: "Alt",
  shift: "Shift",
};

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
  pageup: "⇞",
  pagedown: "⇟",
};

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
  pageup: "PgUp",
  pagedown: "PgDn",
};

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
  return token.toUpperCase();
}

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
