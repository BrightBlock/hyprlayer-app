import type { ITheme } from "@xterm/xterm";

const cssVar = (name: string, fallback: string): string => {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return v || fallback;
};

const ansiVar = (ansi: string, semantic: string | null, fallback: string): string =>
  cssVar(ansi, "") || (semantic ? cssVar(semantic, fallback) : fallback);

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
