import type { ITheme } from "@xterm/xterm";

import type { Rgb, TerminalTheme } from "@/lib/bindings";

const cssVar = (name: string, fallback: string): string => {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return v || fallback;
};

const ansiVar = (ansi: string, semantic: string | null, fallback: string): string =>
  cssVar(ansi, "") || (semantic ? cssVar(semantic, fallback) : fallback);

function parseRgb(color: string): Rgb | null {
  const hex = /^#([0-9a-f]{3}|[0-9a-f]{6}|[0-9a-f]{8})$/i.exec(color.trim())?.[1];
  if (hex) {
    const full = hex.length === 3 ? [...hex].map((c) => c + c).join("") : hex;
    return [0, 2, 4].map((i) => parseInt(full.slice(i, i + 2), 16)) as Rgb;
  }
  const rgb = /^rgba?\(\s*(\d+)[\s,]+(\d+)[\s,]+(\d+)/i.exec(color.trim());
  return rgb ? [Number(rgb[1]), Number(rgb[2]), Number(rgb[3])] : null;
}

/** Claude Code's split of an OSC 11 reply, so a theme report never contradicts one. */
export function isDarkBackground(color: string | undefined): boolean {
  const rgb = parseRgb(color ?? "");
  if (!rgb) return true;
  const [r, g, b] = rgb;
  return 0.2126 * r + 0.7152 * g + 0.0722 * b <= 127.5;
}

const ANSI = [
  "black",
  "red",
  "green",
  "yellow",
  "blue",
  "magenta",
  "cyan",
  "white",
  "brightBlack",
  "brightRed",
  "brightGreen",
  "brightYellow",
  "brightBlue",
  "brightMagenta",
  "brightCyan",
  "brightWhite",
] as const;

export function terminalColors(theme: ITheme): TerminalTheme {
  const dark = isDarkBackground(theme.background);
  const [ink, paper]: Rgb[] = dark
    ? [
        [255, 255, 255],
        [0, 0, 0],
      ]
    : [
        [0, 0, 0],
        [255, 255, 255],
      ];
  const rgb = (color: string | undefined, fallback: Rgb): Rgb => parseRgb(color ?? "") ?? fallback;
  return {
    foreground: rgb(theme.foreground, ink),
    background: rgb(theme.background, paper),
    cursor: rgb(theme.cursor, ink),
    ansi: ANSI.map((slot) => rgb(theme[slot], ink)) as TerminalTheme["ansi"],
    dark,
  };
}

export function xtermThemeFromCss(): ITheme {
  const ink = cssVar("--color-ink", "#e8e6e2");
  const surface = cssVar("--color-surface", "#0c0c0c");
  return {
    background: surface,
    foreground: ink,
    cursor: cssVar("--color-accent", "#d97757"),
    selectionBackground: cssVar("--color-edge", "#333333"),
    black: ansiVar("--color-ansi-black", null, "#2e3436"),
    red: ansiVar("--color-ansi-red", "--color-bad", "#ef5350"),
    green: ansiVar("--color-ansi-green", "--color-ok", "#4caf50"),
    yellow: ansiVar("--color-ansi-yellow", "--color-warn", "#f0b13a"),
    blue: ansiVar("--color-ansi-blue", "--color-brand", "#60a5fa"),
    magenta: ansiVar("--color-ansi-magenta", "--color-accent", "#d97757"),
    cyan: ansiVar("--color-ansi-cyan", "--color-brand-soft", "#7dd3fc"),
    white: ansiVar("--color-ansi-white", "--color-ink", "#e8e6e2"),
    brightBlack: ansiVar("--color-ansi-bright-black", "--color-ink-dim", "#a8a6a2"),
    brightRed: ansiVar("--color-ansi-bright-red", null, "#ef2929"),
    brightGreen: ansiVar("--color-ansi-bright-green", null, "#8ae234"),
    brightYellow: ansiVar("--color-ansi-bright-yellow", null, "#fce94f"),
    brightBlue: ansiVar("--color-ansi-bright-blue", null, "#729fcf"),
    brightMagenta: ansiVar("--color-ansi-bright-magenta", null, "#ad7fa8"),
    brightCyan: ansiVar("--color-ansi-bright-cyan", null, "#34e2e2"),
    brightWhite: ansiVar("--color-ansi-bright-white", null, "#eeeeec"),
  };
}
