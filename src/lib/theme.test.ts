// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OmarchyTheme } from "@/lib/bindings";

// theme.ts imports `commands` from ./bindings as a *value*, and the real
// bindings.ts imports `invoke` from @tauri-apps/api/core, which throws under
// vitest with no Tauri runtime present. Replace the whole module rather than
// letting that import ever execute.
const bindings = vi.hoisted(() => ({ getOmarchyTheme: vi.fn() }));
vi.mock("@/lib/bindings", () => ({
  commands: { getOmarchyTheme: bindings.getOmarchyTheme },
  events: {},
}));

const STORAGE_KEY = "hyprlayer:theme";

// A representative stand-in for the 30 names Phase 2 resolves (the 31
// @theme names minus --color-bad) — this file has no way to see the Rust
// side's exact list, so it mirrors index.css's own token names rather than
// inventing arbitrary ones.
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

// The three shapes get_omarchy_theme can return. GONE and UNREADABLE both
// carry no palette and are handled differently on purpose — see
// refreshOmarchyTheme.
const GONE = { installed: false, theme: null };
const UNREADABLE = { installed: true, theme: null };
const found = (theme: OmarchyTheme) => ({ installed: true, theme });

/** Every custom property name currently set inline on <html>. */
function inlineProps(): string[] {
  const el = document.documentElement;
  const names: string[] = [];
  for (let i = 0; i < el.style.length; i++) names.push(el.style.item(i));
  return names;
}

function seed(value: unknown): void {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(value));
}

/**
 * A fresh module instance. zustand's persist hydrates at module-evaluation
 * time (see theme.ts's merge/migrate), so re-importing statically would only
 * ever hydrate once for the whole file — resetModules is the only way to
 * exercise hydration against a different localStorage seed per test.
 */
async function loadTheme() {
  vi.resetModules();
  return import("@/lib/theme");
}

beforeEach(() => {
  localStorage.clear();
  bindings.getOmarchyTheme.mockReset();
  // The no-op answer, so a test that never sets its own can't be perturbed by
  // the clearing GONE branch.
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
    // jsdom has no matchMedia at all (unlike a real browser), so prefersDark()
    // always resolves false here and the fallback lands on "light" — see
    // terminalTheme.test.ts's own note on jsdom having no real palette.
    expect(document.documentElement.classList.contains("light")).toBe(true);
  });

  it("never sets --color-bad", async () => {
    const { useThemeStore, applyTheme } = await loadTheme();
    useThemeStore.setState({ mode: "omarchy", omarchy: DARK_PALETTE });
    applyTheme();
    expect(inlineProps()).not.toContain("--color-bad");
    expect(document.documentElement.style.getPropertyValue("--color-bad")).toBe("");
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
  // The case that guards the whole auto-select feature: merge(undefined, …)
  // must NOT read as "explicit", or a fresh install can never auto-select
  // Omarchy. See the "existing record" cases below for the trap this guards
  // against — coercing every falsy `explicit` to true, including "absent".
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

  // Distinct from the "no record at all" case above: this record exists (at
  // the current version, so migrate never runs) and simply lacks the field.
  // A naive `persisted?.explicit ?? false` would treat this the same as no
  // record and stay false — wrong, because the record's mode/omarchy are
  // real, previously-persisted values, not an untouched default.
  it("coerces an existing v2 record with a missing explicit flag to true", async () => {
    seed({ state: { mode: "dark" }, version: 2 });
    const { useThemeStore } = await loadTheme();
    expect(useThemeStore.getState().mode).toBe("dark");
    expect(useThemeStore.getState().explicit).toBe(true);
  });

  // The mirror case: a record can legitimately be v2 with explicit: false —
  // it's exactly what the auto-select itself writes (setState({mode:
  // "omarchy"}) without touching explicit). Coercing every falsy value to
  // true, rather than testing `!== false` specifically, would flip this back
  // to true on the very next launch and re-lock the auto-selected theme in as
  // if the user had chosen it by hand.
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

  // setProperty accepts non-custom properties, so a token name is not just a
  // string: without the --color- prefix test, this record would hydrate and
  // applyTheme would set `display: none` on <html>.
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

  // The theme-switch race: Omarchy is installed, colors.toml is momentarily
  // gone mid `rm -rf` + `mv`. Clearing here would drop the user's palette for
  // ~2s on every single theme switch.
  it("leaves a stored palette untouched when installed but unreadable", async () => {
    const { useThemeStore, refreshOmarchyTheme } = await loadTheme();
    useThemeStore.setState({ omarchy: DARK_PALETTE });
    bindings.getOmarchyTheme.mockResolvedValue(UNREADABLE);
    await refreshOmarchyTheme();
    expect(useThemeStore.getState().omarchy).toEqual(DARK_PALETTE);
  });

  // The other half of the same pair, and the one a bare `theme === null`
  // check got wrong: the palette is persisted, so failing to clear it here
  // repaints an uninstalled theme on every relaunch, forever.
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

  // The mirror: a hand-picked "Omarchy" is a deliberate choice and survives an
  // uninstall — applyTheme's null branch already paints a sane fallback, and
  // re-installing brings the palette straight back.
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
    // The auto-select is not a user choice: it must not lock itself in.
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
