import type { ITheme } from "@xterm/xterm";

const cssVar = (name: string, fallback: string): string => {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return v || fallback;
};

/** Read an ANSI slot. `semantic` is the token that drove this slot before the
 *  --color-ansi-* family existed, for the eight that had one; the other eight
 *  pass null and fall straight to their stock-Tango literal. The @theme
 *  defaults already express both fallbacks in CSS; this repeats them in JS so
 *  the vitest suite — which loads no stylesheet at all — keeps asserting the
 *  same values it does today.
 *
 *  Every `ansi` argument at the sixteen call sites below must stay a literal
 *  string. Tailwind v4's oxide scanner keeps a `@theme` custom property in
 *  the built stylesheet only by finding its bare name in a scanned source
 *  file — a `@theme` declaration whose value is `var(--X)` registers `--X`
 *  as a dependency, it does not mark the declaring variable itself used
 *  (the same mechanism NEBULA_COLORS relies on in
 *  features/dashboard/graph.ts:30-34). Building the property name
 *  dynamically here (`` `--color-ansi-${slot}` ``) would silently drop all
 *  sixteen `--color-ansi-*` declarations from the stylesheet, and the only
 *  symptom would be every terminal rendering stock Tango. */
const ansiVar = (ansi: string, semantic: string | null, fallback: string): string =>
  cssVar(ansi, "") || (semantic ? cssVar(semantic, fallback) : fallback);

/** Map hyprlayer theme tokens (src/index.css) onto an xterm palette. */
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
