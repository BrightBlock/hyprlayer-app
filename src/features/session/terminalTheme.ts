import type { ITheme } from "@xterm/xterm";

const cssVar = (name: string, fallback: string): string => {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return v || fallback;
};

/** Map hyprlayer theme tokens (src/index.css) onto an xterm palette. */
export function xtermThemeFromCss(): ITheme {
  const ink = cssVar("--color-ink", "#e8e6e2");
  const surface = cssVar("--color-surface", "#0c0c0c");
  return {
    background: surface,
    foreground: ink,
    cursor: cssVar("--color-accent", "#d97757"),
    selectionBackground: cssVar("--color-edge", "#333333"),
    red: cssVar("--color-bad", "#ef5350"),
    green: cssVar("--color-ok", "#4caf50"),
    yellow: cssVar("--color-warn", "#f0b13a"),
    blue: cssVar("--color-brand", "#60a5fa"),
    cyan: cssVar("--color-brand-soft", "#7dd3fc"),
    magenta: cssVar("--color-accent", "#d97757"),
    white: ink,
    brightBlack: cssVar("--color-ink-dim", "#a8a6a2"),
    // bright* defaults to base where not separately themed
  };
}
