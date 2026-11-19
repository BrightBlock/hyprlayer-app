// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OmarchyTheme } from "@/lib/bindings";

const bindings = vi.hoisted(() => ({ getOmarchyTheme: vi.fn() }));
vi.mock("@/lib/bindings", () => ({
  commands: { getOmarchyTheme: bindings.getOmarchyTheme },
  events: {},
}));

const STORAGE_KEY = "hyprlayer:theme";

const TOKEN_NAMES = [
  "surface",
  "panel",
  "panel-2",
  "edge",
  "ink",
  "ink-dim",
  "accent",
  "accent-ink",
  "ok",
  "warn",
  "brand",
  "brand-soft",
  "attention",
  "ship",
  "commit",
  "research",
  "plan",
  "implement",
  "validate",
  "revise",
  "handoff",
  "working",
  "background",
  "nebula-bg-0",
  "nebula-bg-1",
  "nebula-1",
  "nebula-2",
  "nebula-3",
  "nebula-4",
  "nebula-5",
].map((n) => `--color-${n}`);

function palette(mode: "dark" | "light", name = "diablo-dreams"): OmarchyTheme {
  return {
    name,
    mode,
    tokens: TOKEN_NAMES.map((tokenName, i) => ({
      name: tokenName,
      value: `#${(i + 1).toString(16).padStart(6, "0")}`,
    })),
  };
}

const DARK_PALETTE = palette("dark");
const LIGHT_PALETTE = palette("light", "white");

const GONE = { installed: false, theme: null };
const UNREADABLE = { installed: true, theme: null };
const found = (theme: OmarchyTheme) => ({ installed: true, theme });

function inlineProps(): string[] {
  const el = document.documentElement;
  const names: string[] = [];
  for (let i = 0; i < el.style.length; i++) names.push(el.style.item(i));
  return names;
}

function seed(value: unknown): void {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(value));
}

async function loadTheme() {
  vi.resetModules();
  return import("@/lib/theme");
}

beforeEach(() => {
  localStorage.clear();
  bindings.getOmarchyTheme.mockReset();
  bindings.getOmarchyTheme.mockResolvedValue(UNREADABLE);
});

afterEach(() => {
  document.documentElement.className = "";
  document.documentElement.removeAttribute("style");
});

describe("applyTheme", () => {
  it("sets all 30 tokens inline and adds no class for a dark Omarchy palette", async () => {
    const { useThemeStore, applyTheme } = await loadTheme();
    useThemeStore.setState({ mode: "omarchy", omarchy: DARK_PALETTE });
    applyTheme();
    expect(new Set(inlineProps())).toEqual(new Set(TOKEN_NAMES));
    expect(document.documentElement.style.getPropertyValue("--color-surface")).toBe(
      DARK_PALETTE.tokens.find((t) => t.name === "--color-surface")?.value,
    );
    expect(document.documentElement.classList.contains("light")).toBe(false);
  });

  it("adds the light class and still sets all 30 for a light Omarchy palette", async () => {
    const { useThemeStore, applyTheme } = await loadTheme();
    useThemeStore.setState({ mode: "omarchy", omarchy: LIGHT_PALETTE });
    applyTheme();
    expect(document.documentElement.classList.contains("light")).toBe(true);
    expect(new Set(inlineProps())).toEqual(new Set(TOKEN_NAMES));
  });

  it("removes every previously-set property and the class when switching away from omarchy", async () => {
    const { useThemeStore, applyTheme } = await loadTheme();
    useThemeStore.setState({ mode: "omarchy", omarchy: LIGHT_PALETTE });
    applyTheme();
    expect(document.documentElement.style.length).toBeGreaterThan(0);
    expect(document.documentElement.classList.contains("light")).toBe(true);

    useThemeStore.setState({ mode: "dark" });
    applyTheme();
    expect(document.documentElement.style.length).toBe(0);
    expect(document.documentElement.classList.contains("light")).toBe(false);
  });

  it("sets nothing and falls back to the prefersDark() resolution when the palette is null", async () => {
    const { useThemeStore, applyTheme } = await loadTheme();
    useThemeStore.setState({ mode: "omarchy", omarchy: null });
    applyTheme();
    expect(document.documentElement.style.length).toBe(0);
    expect(document.documentElement.classList.contains("light")).toBe(true);
  });

});

describe("appearanceSegment", () => {
  it("maps the three OS-scheme modes onto themselves", async () => {
    const { appearanceSegment } = await loadTheme();
    expect(appearanceSegment("light")).toBe("light");
    expect(appearanceSegment("dark")).toBe("dark");
    expect(appearanceSegment("system")).toBe("system");
  });

  it("selects no segment for a named theme, never passing it off as dark", async () => {
    const { appearanceSegment } = await loadTheme();
    for (const mode of ["claude", "cobalt", "matrix", "solarized", "omarchy"] as const) {
      expect(appearanceSegment(mode)).toBeNull();
    }
  });
});

describe("setMode", () => {
  it("marks the choice explicit", async () => {
    const { useThemeStore } = await loadTheme();
    expect(useThemeStore.getState().explicit).toBe(false);
    useThemeStore.getState().setMode("dark");
    expect(useThemeStore.getState().explicit).toBe(true);
  });
});

describe("hydration", () => {
  it("yields explicit: false with no persisted record at all", async () => {
    expect(localStorage.getItem(STORAGE_KEY)).toBe(null);
    const { useThemeStore } = await loadTheme();
    expect(useThemeStore.getState().explicit).toBe(false);
    expect(useThemeStore.getState().omarchy).toBe(null);
    expect(useThemeStore.getState().mode).toBe("system");
  });

  it("migrates a v1 record (no explicit field, version 1) to explicit: true", async () => {
    seed({ state: { mode: "system" }, version: 1 });
    const { useThemeStore } = await loadTheme();
    expect(useThemeStore.getState().mode).toBe("system");
    expect(useThemeStore.getState().explicit).toBe(true);
  });

  it("coerces an existing v2 record with a missing explicit flag to true", async () => {
    seed({ state: { mode: "dark" }, version: 2 });
    const { useThemeStore } = await loadTheme();
    expect(useThemeStore.getState().mode).toBe("dark");
    expect(useThemeStore.getState().explicit).toBe(true);
  });

  it("keeps an existing explicit: false record false", async () => {
    seed({ state: { mode: "omarchy", explicit: false, omarchy: DARK_PALETTE }, version: 2 });
    const { useThemeStore } = await loadTheme();
    expect(useThemeStore.getState().mode).toBe("omarchy");
    expect(useThemeStore.getState().explicit).toBe(false);
    expect(useThemeStore.getState().omarchy).toEqual(DARK_PALETTE);
  });

  it("drops a structurally invalid persisted omarchy value", async () => {
    seed({ state: { mode: "dark", omarchy: { garbage: true } }, version: 2 });
    const { useThemeStore } = await loadTheme();
    expect(useThemeStore.getState().omarchy).toBe(null);
  });

  it("drops a persisted omarchy value whose tokens are not --color-* names", async () => {
    seed({
      state: {
        mode: "omarchy",
        omarchy: { ...DARK_PALETTE, tokens: [{ name: "display", value: "none" }] },
      },
      version: 2,
    });
    const { useThemeStore, applyTheme } = await loadTheme();
    expect(useThemeStore.getState().omarchy).toBe(null);
    applyTheme();
    expect(document.documentElement.style.getPropertyValue("display")).toBe("");
  });

  it("falls back to the in-memory mode on an unknown persisted id", async () => {
    seed({ state: { mode: "not-a-real-theme" }, version: 2 });
    const { useThemeStore } = await loadTheme();
    expect(useThemeStore.getState().mode).toBe("system");
  });
});

describe("refreshOmarchyTheme", () => {
  it("stores a resolved palette", async () => {
    const { useThemeStore, refreshOmarchyTheme } = await loadTheme();
    bindings.getOmarchyTheme.mockResolvedValue(found(DARK_PALETTE));
    await refreshOmarchyTheme();
    expect(useThemeStore.getState().omarchy).toEqual(DARK_PALETTE);
  });

  it("leaves a stored palette untouched when installed but unreadable", async () => {
    const { useThemeStore, refreshOmarchyTheme } = await loadTheme();
    useThemeStore.setState({ omarchy: DARK_PALETTE });
    bindings.getOmarchyTheme.mockResolvedValue(UNREADABLE);
    await refreshOmarchyTheme();
    expect(useThemeStore.getState().omarchy).toEqual(DARK_PALETTE);
  });

  it("clears a stored palette when Omarchy is not installed", async () => {
    const { useThemeStore, refreshOmarchyTheme } = await loadTheme();
    useThemeStore.setState({ omarchy: DARK_PALETTE });
    bindings.getOmarchyTheme.mockResolvedValue(GONE);
    await refreshOmarchyTheme();
    expect(useThemeStore.getState().omarchy).toBe(null);
  });

  it("walks back an auto-selected Omarchy mode when Omarchy is not installed", async () => {
    const { useThemeStore, refreshOmarchyTheme } = await loadTheme();
    useThemeStore.setState({ mode: "omarchy", explicit: false, omarchy: DARK_PALETTE });
    bindings.getOmarchyTheme.mockResolvedValue(GONE);
    await refreshOmarchyTheme();
    expect(useThemeStore.getState().mode).toBe("system");
    expect(useThemeStore.getState().explicit).toBe(false);
  });

  it("keeps a deliberate Omarchy mode when Omarchy is not installed", async () => {
    const { useThemeStore, refreshOmarchyTheme } = await loadTheme();
    useThemeStore.getState().setMode("omarchy");
    useThemeStore.setState({ omarchy: DARK_PALETTE });
    bindings.getOmarchyTheme.mockResolvedValue(GONE);
    await refreshOmarchyTheme();
    expect(useThemeStore.getState().mode).toBe("omarchy");
    expect(useThemeStore.getState().omarchy).toBe(null);
  });

  it("auto-selects Omarchy when explicit is false and mode is system", async () => {
    const { useThemeStore, refreshOmarchyTheme } = await loadTheme();
    expect(useThemeStore.getState().mode).toBe("system");
    expect(useThemeStore.getState().explicit).toBe(false);
    bindings.getOmarchyTheme.mockResolvedValue(found(DARK_PALETTE));
    await refreshOmarchyTheme();
    expect(useThemeStore.getState().mode).toBe("omarchy");
    expect(useThemeStore.getState().explicit).toBe(false);
  });

  it("does not auto-select when explicit is true, even on mode system", async () => {
    const { useThemeStore, refreshOmarchyTheme } = await loadTheme();
    useThemeStore.setState({ mode: "system", explicit: true });
    bindings.getOmarchyTheme.mockResolvedValue(found(DARK_PALETTE));
    await refreshOmarchyTheme();
    expect(useThemeStore.getState().mode).toBe("system");
    expect(useThemeStore.getState().omarchy).toEqual(DARK_PALETTE);
  });

  it("does not auto-select when mode is already a deliberate non-system choice", async () => {
    const { useThemeStore, refreshOmarchyTheme } = await loadTheme();
    useThemeStore.getState().setMode("dark");
    bindings.getOmarchyTheme.mockResolvedValue(found(DARK_PALETTE));
    await refreshOmarchyTheme();
    expect(useThemeStore.getState().mode).toBe("dark");
  });
});
