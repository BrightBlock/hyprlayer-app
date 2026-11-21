// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { terminalColors, xtermThemeFromCss } from "./terminalTheme";

const ANSI_SLOTS = [
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

describe("xtermThemeFromCss", () => {
  afterEach(() => {
    document.documentElement.removeAttribute("style");
  });

  it("reads CSS custom properties when explicitly set on documentElement", () => {
    document.documentElement.style.setProperty("--color-ink", "#aabbcc");
    document.documentElement.style.setProperty("--color-surface", "#112233");
    document.documentElement.style.setProperty("--color-accent", "#ffcc00");
    const theme = xtermThemeFromCss();
    expect(theme.foreground).toBe("#aabbcc");
    expect(theme.background).toBe("#112233");
    expect(theme.cursor).toBe("#ffcc00");
    expect(theme.white).toBe("#aabbcc");
  });

  it("fills the base colors and all 16 ANSI slots with their documented fallback when no vars are set", () => {
    const theme = xtermThemeFromCss();
    expect(theme.foreground).toBe("#e8e6e2");
    expect(theme.background).toBe("#0c0c0c");
    expect(theme.cursor).toBe("#d97757");
    expect(theme.black).toBe("#2e3436");
    expect(theme.brightRed).toBe("#ef2929");
    expect(theme.brightGreen).toBe("#8ae234");
    expect(theme.brightYellow).toBe("#fce94f");
    expect(theme.brightBlue).toBe("#729fcf");
    expect(theme.brightMagenta).toBe("#ad7fa8");
    expect(theme.brightCyan).toBe("#34e2e2");
    expect(theme.brightWhite).toBe("#eeeeec");
    expect(theme.red).toBe("#ef5350");
    expect(theme.green).toBe("#4caf50");
    expect(theme.yellow).toBe("#f0b13a");
    expect(theme.blue).toBe("#60a5fa");
    expect(theme.magenta).toBe("#d97757");
    expect(theme.cyan).toBe("#7dd3fc");
    expect(theme.white).toBe("#e8e6e2");
    expect(theme.brightBlack).toBe("#a8a6a2");
    for (const slot of ANSI_SLOTS) expect(theme[slot], slot).toBeTruthy();
  });

  it("--color-ansi-red follows --color-bad when only the semantic token is set", () => {
    document.documentElement.style.setProperty("--color-bad", "#ff00ff");
    const theme = xtermThemeFromCss();
    expect(theme.red).toBe("#ff00ff");
  });

  it("--color-ansi-red wins over --color-bad when both are set", () => {
    document.documentElement.style.setProperty("--color-bad", "#ff00ff");
    document.documentElement.style.setProperty("--color-ansi-red", "#123456");
    const theme = xtermThemeFromCss();
    expect(theme.red).toBe("#123456");
    for (const slot of ANSI_SLOTS) expect(theme[slot], slot).toBeTruthy();
  });
});

describe("terminalColors", () => {
  afterEach(() => {
    document.documentElement.removeAttribute("style");
  });

  it("gives the server the theme's colours as RGB, ANSI slots in order, with its darkness", () => {
    document.documentElement.style.setProperty("--color-ink", "#111111");
    document.documentElement.style.setProperty("--color-surface", "rgb(247, 247, 247)");
    document.documentElement.style.setProperty("--color-ansi-red", "#c0392b");
    const colors = terminalColors(xtermThemeFromCss());
    expect(colors.foreground).toEqual([0x11, 0x11, 0x11]);
    expect(colors.background).toEqual([247, 247, 247]);
    expect(colors.cursor).toEqual([0xd9, 0x77, 0x57]);
    expect(colors.ansi).toHaveLength(16);
    expect(colors.ansi[1]).toEqual([0xc0, 0x39, 0x2b]);
    expect(colors.ansi[15]).toEqual([0xee, 0xee, 0xec]);
    expect(colors.dark).toBe(false);
  });

  it("reads a colour it can't parse as the theme's plain ink or paper", () => {
    document.documentElement.style.setProperty("--color-ansi-red", "oklch(0.6 0.2 25)");
    document.documentElement.style.setProperty("--color-surface", "oklch(0.1 0 0)");
    const colors = terminalColors(xtermThemeFromCss());
    expect(colors.dark).toBe(true);
    expect(colors.ansi[1]).toEqual([0xff, 0xff, 0xff]);
    expect(colors.background).toEqual([0, 0, 0]);
  });
});
