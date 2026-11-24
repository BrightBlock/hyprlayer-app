import { describe, expect, it } from "vitest";

// Vitest serves stylesheets empty, even with ?raw, so this reads the file from disk.
const fs = await import("node:" + "fs");
const CSS: string = fs.readFileSync("src/index.css", "utf8");

const TSX_SOURCES = import.meta.glob<string>("../**/*.tsx", {
  eager: true,
  import: "default",
  query: "?raw",
});

// ADR-0006: WebKitGTK composites these. Anything else repaints on every frame it animates, and
// without the DMA-BUF renderer every frame repaints the whole window on the webview's main thread.
const COMPOSITED = new Set(["transform", "translate", "rotate", "scale", "opacity"]);

// Tailwind's own keyframes; each animates only transform or opacity.
const TAILWIND_ANIMATIONS = new Set(["spin", "ping", "pulse", "bounce"]);

// Tailwind's own endless animations, used in components without a rule in index.css.
const TAILWIND_ENDLESS = [".animate-spin", ".animate-pulse", ".animate-ping"];

const TRANSITION_CLASSES: Record<string, string[]> = {
  "transition-opacity": ["opacity"],
  "transition-transform": ["transform", "translate", "scale", "rotate"],
  "transition-none": [],
};

function blockAfter(source: string, open: number): string {
  let depth = 0;
  for (let i = open; i < source.length; i++) {
    if (source[i] === "{") depth++;
    else if (source[i] === "}" && --depth === 0) return source.slice(open + 1, i);
  }
  throw new Error("unbalanced braces");
}

function selectorBefore(index: number): string {
  const open = CSS.lastIndexOf("{", index);
  const start = Math.max(CSS.lastIndexOf("}", open), CSS.lastIndexOf("*/", open)) + 1;
  return CSS.slice(start, open).trim();
}

function keyframeProperties(frames: string): string[] {
  return [...frames.matchAll(/\{([^{}]*)\}/g)].flatMap((step) =>
    step[1]
      .split(";")
      .map((decl) => decl.split(":")[0].trim())
      .filter(Boolean),
  );
}

function transitionProperties(value: string): string[] {
  return value
    .split(",")
    .map((part) => part.trim().split(/\s+/)[0])
    .filter((prop) => prop && prop !== "none" && !/^[\d.]+m?s$/.test(prop));
}

const lineOf = (source: string, index: number) => source.slice(0, index).split("\n").length;
const fileOf = (path: string) => path.replace(/^\.\.\//, "src/");

const keyframes = new Map<string, string[]>(
  [...CSS.matchAll(/@keyframes\s+([\w-]+)\s*\{/g)].map((m) => [
    m[1],
    keyframeProperties(blockAfter(CSS, m.index + m[0].length - 1)),
  ]),
);

const endless = [...CSS.matchAll(/animation:\s*([\w-]+)[^;]*\binfinite\b/g)].map((m) => ({
  keyframes: m[1],
  selector: selectorBefore(m.index),
}));

describe("keyframes and stylesheet transitions", () => {
  it("animate only properties the compositor can move", () => {
    expect(keyframes.size).toBeGreaterThan(0);
    const painted = [...keyframes].flatMap(([name, props]) =>
      [...new Set(props)].filter((p) => !COMPOSITED.has(p)).map((p) => `@keyframes ${name} animates ${p}`),
    );
    expect(painted).toEqual([]);
  });

  it("transition only properties the compositor can move", () => {
    const painted = [...CSS.matchAll(/(?<![\w-])transition(?:-property)?:\s*([^;]+);/g)].flatMap((m) =>
      transitionProperties(m[1])
        .filter((p) => !COMPOSITED.has(p))
        .map((p) => `src/index.css:${lineOf(CSS, m.index)} transitions ${p}`),
    );
    expect(painted).toEqual([]);
  });
});

describe("component motion", () => {
  const sources = Object.entries(TSX_SOURCES).filter(([file]) => !file.includes(".test."));

  it("transitions only properties the compositor can move", () => {
    const offenders: string[] = [];
    let seen = 0;
    for (const [file, source] of sources) {
      for (const m of source.matchAll(/(?<=[\s"'`:])(transition(?:-\[[^\]\s]+\]|-[a-z]+)?)(?=[\s"'`])/g)) {
        seen++;
        const arbitrary = m[1].match(/^transition-\[(.+)\]$/);
        const props = arbitrary ? arbitrary[1].split(",") : TRANSITION_CLASSES[m[1]];
        const painted = props ? props.filter((p) => !COMPOSITED.has(p)) : ["paint properties"];
        if (painted.length) offenders.push(`${fileOf(file)}:${lineOf(source, m.index)} ${m[1]} transitions ${painted.join(", ")}`);
      }
      // Inline style transitions, including ones assembled from strings before reaching `transition`.
      for (const m of source.matchAll(/["'`]([a-z-]+(?:\s+(?:[\d.]+m?s|var\([^)]*\)|calc\([^)]*\)[^,"'`]*))[^"'`]*)["'`]/g)) {
        const painted = transitionProperties(m[1]).filter((p) => /^[a-z-]+$/.test(p) && !COMPOSITED.has(p));
        if (painted.length) offenders.push(`${fileOf(file)}:${lineOf(source, m.index)} inline transition on ${painted.join(", ")}`);
      }
    }
    expect(seen).toBeGreaterThan(0);
    expect(offenders).toEqual([]);
  });

  it("names only known animations", () => {
    const unknown = sources.flatMap(([file, source]) =>
      [...source.matchAll(/(?<=[\s"'`:])animate-(\[[^\]\s]+\]|[\w-]+)(?=[\s"'`])/g)]
        .filter((m) => !TAILWIND_ANIMATIONS.has(m[1]) && !keyframes.has(m[1]))
        .map((m) => `${fileOf(file)}:${lineOf(source, m.index)} animate-${m[1]}`),
    );
    expect(unknown).toEqual([]);
  });
});

describe("endless animations", () => {
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
      .map((m) => blockAfter(CSS, m.index + m[0].length - 1))
      .filter((block) => /animation:\s*none/.test(block))
      .join(" ");
    expect([...endless.map((e) => e.selector), ...TAILWIND_ENDLESS].filter((cls) => !reduced.includes(cls))).toEqual([]);
  });

  it("all hold still where WebKitGTK repaints the whole window per frame", () => {
    expect(unlisted(ruleFor("data-still-motion", /animation:\s*none/g))).toEqual([]);
  });
});

// The guided tour moves once, on entrance, and not at all where motion is held still or reduced.
describe("guided tour motion", () => {
  const TOUR_CLASSES = [".tour-in"];
  const stillRules = [...CSS.matchAll(/animation:\s*none/g)].map((m) => selectorBefore(m.index));
  const reducedBlocks = [...CSS.matchAll(/@media\s*\(prefers-reduced-motion:\s*reduce\)\s*\{/g)]
    .map((m) => blockAfter(CSS, m.index + m[0].length - 1))
    .filter((block) => /animation:\s*none/.test(block))
    .join(" ");

  it("never loops", () => {
    expect(endless.filter((e) => TOUR_CLASSES.some((c) => e.selector.includes(c)))).toEqual([]);
    expect(keyframes.get("tour-in")).toBeDefined();
  });

  it("holds still under data-still-motion", () => {
    const still = stillRules.filter((s) => s.includes("[data-still-motion]")).join(" ");
    expect(TOUR_CLASSES.filter((c) => !still.includes(c))).toEqual([]);
  });

  it("holds still for reduced motion", () => {
    expect(TOUR_CLASSES.filter((c) => !reducedBlocks.includes(c))).toEqual([]);
  });

  it("is the only motion on the coach mark, its highlight, the intro scrim and the second outline", () => {
    const coach = Object.entries(TSX_SOURCES).find(([f]) => f.endsWith("tour/CoachMark.tsx"))?.[1] ?? "";
    expect(coach.match(/\btour-in\b/g)?.length).toBeGreaterThanOrEqual(5);
    expect(coach).not.toMatch(/\b(animate-|transition)/);
  });

  it("is the card's only motion", () => {
    const card = Object.entries(TSX_SOURCES).find(([f]) => f.endsWith("tour/WhatsNewCard.tsx"))?.[1] ?? "";
    expect(card).toMatch(/\btour-in\b/);
    expect(card).not.toMatch(/\b(animate-|transition)/);
  });
});
