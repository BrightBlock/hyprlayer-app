// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { xtermThemeFromCss } from "./terminalTheme";

describe("xtermThemeFromCss", () => {
  afterEach(() => {
    // Reset any inline vars between tests.
    document.documentElement.removeAttribute("style");
  });

  it("returns fallback values when CSS vars are absent (jsdom has no real palette)", () => {
    const theme = xtermThemeFromCss();
    expect(theme.foreground).toBe("#e8e6e2");
    expect(theme.background).toBe("#0c0c0c");
    expect(theme.cursor).toBe("#d97757");
    expect(theme.red).toBe("#ef5350");
    expect(theme.green).toBe("#4caf50");
    expect(theme.yellow).toBe("#f0b13a");
    expect(theme.blue).toBe("#60a5fa");
    expect(theme.cyan).toBe("#7dd3fc");
    expect(theme.magenta).toBe("#d97757");
    expect(theme.white).toBe("#e8e6e2");
    expect(theme.brightBlack).toBe("#a8a6a2");
  });

  it("reads CSS custom properties when explicitly set on documentElement", () => {
    document.documentElement.style.setProperty("--color-ink", "#aabbcc");
    document.documentElement.style.setProperty("--color-surface", "#112233");
    document.documentElement.style.setProperty("--color-accent", "#ffcc00");
    const theme = xtermThemeFromCss();
    expect(theme.foreground).toBe("#aabbcc");
    expect(theme.background).toBe("#112233");
    expect(theme.cursor).toBe("#ffcc00");
    // --color-ink drives both foreground and white
    expect(theme.white).toBe("#aabbcc");
  });
});
