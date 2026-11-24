import { describe, expect, it } from "vitest";

// Vitest serves stylesheets empty, even with ?raw, so this reads the file from disk.
const fs = await import("node:" + "fs");
const CSS: string = fs.readFileSync("src/index.css", "utf8");

const COMPOSITED = new Set(["transform", "translate", "rotate", "scale", "opacity"]);

// Tailwind's own endless animations, used in components without a rule in index.css.
const TAILWIND_ENDLESS = [".animate-spin", ".animate-pulse", ".animate-ping"];

function blockAfter(open: number): string {
  let depth = 0;
  for (let i = open; i < CSS.length; i++) {
    if (CSS[i] === "{") depth++;
    else if (CSS[i] === "}" && --depth === 0) return CSS.slice(open + 1, i);
  }
  throw new Error("unbalanced braces");
}

function selectorBefore(index: number): string {
  const open = CSS.lastIndexOf("{", index);
  const start = Math.max(CSS.lastIndexOf("}", open), CSS.lastIndexOf("*/", open)) + 1;
  return CSS.slice(start, open).trim();
}

const endless = [...CSS.matchAll(/animation:\s*([\w-]+)[^;]*\binfinite\b/g)].map((m) => ({
  keyframes: m[1],
  selector: selectorBefore(m.index),
}));

describe("endless animations", () => {
  it("animate only properties the compositor can move", () => {
    expect(endless.length).toBeGreaterThan(0);
    const painted = endless.flatMap(({ keyframes }) => {
      const at = CSS.search(new RegExp(`@keyframes\\s+${keyframes}\\s*\\{`));
      expect(at, `@keyframes ${keyframes}`).toBeGreaterThanOrEqual(0);
      const props = [...blockAfter(CSS.indexOf("{", at)).matchAll(/([\w-]+)\s*:/g)].map((m) => m[1]);
      return [...new Set(props)].filter((p) => !COMPOSITED.has(p)).map((p) => `${keyframes} animates ${p}`);
    });
    expect(painted).toEqual([]);
  });

  const ruleFor = (flag: string, decl: RegExp) =>
    [...CSS.matchAll(decl)]
      .map((m) => selectorBefore(m.index))
      .filter((s) => s.includes(`[${flag}]`))
      .join(" ");
  const unlisted = (selectors: string) =>
    [...endless.map((e) => e.selector), ...TAILWIND_ENDLESS].filter((cls) => !selectors.includes(cls));

  it("all pause while the window is in the background", () => {
    expect(unlisted(ruleFor("data-window-blurred", /animation-play-state:\s*paused/g))).toEqual([]);
  });

  it("all stop for reduced motion", () => {
    const reduced = [...CSS.matchAll(/@media\s*\(prefers-reduced-motion:\s*reduce\)\s*\{/g)]
      .map((m) => blockAfter(m.index + m[0].length - 1))
      .filter((block) => /animation:\s*none/.test(block))
      .join(" ");
    expect(endless.map((e) => e.selector).filter((cls) => !reduced.includes(cls))).toEqual([]);
  });

  it("all hold still where WebKitGTK repaints the whole window per frame", () => {
    expect(unlisted(ruleFor("data-still-motion", /animation:\s*none/g))).toEqual([]);
  });
});
