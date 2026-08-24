/**
 * Color themes.
 *
 * A theme is just a set of CSS variables (defined in index.css under either
 * `@theme` for the default Dark+ palette or an `html.<name>` block). This
 * module owns the *choice* of theme: it persists the user's selection and
 * toggles the matching class on <html> so the variables, and therefore every
 * `bg-surface` / `text-ink` / `border-edge` utility, re-resolve.
 */

import { create } from "zustand";
import { persist } from "zustand/middleware";

import { commands, type OmarchyTheme } from "./bindings";

/** "system" follows the OS appearance; "omarchy" follows the Linux desktop's
 *  live palette (see refreshOmarchyTheme, below); the rest map to a concrete
 *  palette. */
export type ThemeMode =
  | "system"
  | "omarchy"
  | "dark"
  | "light"
  | "claude"
  | "cobalt"
  | "matrix"
  | "solarized";

/** What `system`/`omarchy` resolve to, and the set of palettes index.css
 *  defines. */
export type ResolvedTheme =
  | "omarchy"
  | "dark"
  | "light"
  | "claude"
  | "cobalt"
  | "matrix"
  | "solarized";

/** Picker order + labels for the theme menu. */
export const THEMES: ReadonlyArray<{ id: ThemeMode; label: string }> = [
  { id: "system", label: "System" },
  { id: "omarchy", label: "Omarchy" },
  { id: "light", label: "Light" },
  { id: "claude", label: "Claude" },
  { id: "dark", label: "Dark+" },
  { id: "solarized", label: "Solarized Dark" },
  { id: "cobalt", label: "Cobalt" },
  { id: "matrix", label: "Matrix" },
];

// THEMES is the source of truth for VALID_MODES and THEME_CLASSES below, but
// ThemeMode and ResolvedTheme are hand-written unions, not derived from it —
// adding a theme means editing THEMES and both unions by hand.
const VALID_MODES: ReadonlyArray<ThemeMode> = THEMES.map((t) => t.id);
/** Palettes that need a class on <html>. "system" and "omarchy" resolve
 *  elsewhere ("omarchy" to inline properties set by applyTheme, never a
 *  class) and "dark" is the bare `@theme` default (no class), selecting it
 *  just clears the rest. */
const THEME_CLASSES = VALID_MODES.filter(
  (id): id is ResolvedTheme => id !== "system" && id !== "dark" && id !== "omarchy",
);

const prefersDark = () =>
  typeof window !== "undefined" &&
  typeof window.matchMedia === "function" &&
  window.matchMedia("(prefers-color-scheme: dark)").matches;

/** Resolve the user's choice to a concrete palette. `system` checks the OS;
 *  `omarchy` resolves to itself — applyTheme reads the stored palette. */
export function resolveTheme(mode: ThemeMode): ResolvedTheme {
  if (mode === "system") return prefersDark() ? "dark" : "light";
  return mode;
}

/** Structural check for a persisted `omarchy` field. `persisted` in `merge`
 *  below is cast, not verified, so a corrupted or foreign localStorage value
 *  must not reach applyTheme() and start calling setProperty with garbage.
 *
 *  The `--color-` prefix test is the part that isn't merely structural:
 *  setProperty accepts non-custom properties, so a record carrying
 *  `{ name: "display", value: "none" }` would otherwise pass this guard and
 *  applyTheme would blank the app. Deliberately NOT a check against the exact
 *  46 names core emits — hardcoding that list here would break the day core
 *  emits a 47th, which is the same reason appliedTokens is built from the
 *  response rather than a constant. */
function isOmarchyTheme(value: unknown): value is OmarchyTheme {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.name === "string" &&
    (v.mode === "dark" || v.mode === "light") &&
    Array.isArray(v.tokens) &&
    v.tokens.every((t) => {
      if (typeof t !== "object" || t === null) return false;
      const { name, value } = t as Record<string, unknown>;
      return typeof name === "string" && name.startsWith("--color-") && typeof value === "string";
    })
  );
}

type ThemeStore = {
  /** The user's selection (may be "system"). Defaults to "system" so the app
   *  follows the OS light/dark setting; a persisted choice (below) overrides it
   *  for users who've picked another. */
  mode: ThemeMode;
  /** True once the user has picked a theme by hand — including "System".
   *  Gates the Omarchy first-run auto-select, which must never overwrite a
   *  deliberate choice. Absent from v1, where `mode: "system"` was
   *  indistinguishable between "chose it" and "never opened the picker". */
  explicit: boolean;
  /** Last resolved Omarchy palette. Persisted so a relaunch paints it before
   *  the IPC round-trip lands — the same no-flash requirement initTheme()
   *  already serves (main.tsx:24-26). Null off Omarchy, and cleared back to
   *  null by refreshOmarchyTheme the first time it sees Omarchy gone. */
  omarchy: OmarchyTheme | null;
  setMode: (mode: ThemeMode) => void;
  /** `null` clears it — the Omarchy-is-gone case, see refreshOmarchyTheme. */
  setOmarchy: (theme: OmarchyTheme | null) => void;
};

export const useThemeStore = create<ThemeStore>()(
  persist(
    (set) => ({
      mode: "system",
      explicit: false,
      omarchy: null,
      // Every path through the picker marks the choice deliberate.
      setMode: (mode) => set({ mode, explicit: true }),
      setOmarchy: (theme) => set({ omarchy: theme }),
    }),
    {
      name: "hyprlayer:theme",
      version: 2,
      // v1 had no `explicit` flag, so we cannot know whether its "system" was
      // chosen or defaulted. Treat every v1 record as deliberate. The failure
      // mode of the other choice is silently changing an existing user's
      // theme on upgrade; the failure mode of this one is an existing Omarchy
      // user picking "Omarchy" from the menu once. Take the second.
      migrate: (persisted, _from) => ({
        ...(persisted as object),
        explicit: true,
      }),
      // Drop unknown ids (e.g. a theme removed in a later version) so we fall
      // back to the default rather than landing on the bare @theme silently.
      merge: (persisted, current) => {
        const p = persisted as Partial<ThemeStore> | undefined;
        const mode = VALID_MODES.includes(p?.mode as ThemeMode)
          ? (p!.mode as ThemeMode)
          : current.mode;
        return {
          ...current,
          mode,
          // `persisted === undefined` means no stored record at all — a
          // genuine first run, which must stay `explicit: false` so the
          // Omarchy auto-select can fire. Only an EXISTING record with a
          // missing/garbage flag is coerced to true (see the migrate above: a
          // v1 record is deliberate).
          explicit: p === undefined ? current.explicit : p.explicit !== false,
          omarchy: isOmarchyTheme(p?.omarchy) ? p!.omarchy : null,
        };
      },
    },
  ),
);

// Names set on the last call, so switching away removes exactly what we set
// and nothing else. A hardcoded list would silently leak a token the day core
// starts emitting a new one.
let appliedTokens: string[] = [];

/** Toggle the resolved palette's class on <html> and, for Omarchy, its inline
 *  --color-* properties. Safe to call repeatedly. */
export function applyTheme(): void {
  if (typeof document === "undefined") return;
  const { mode, omarchy } = useThemeStore.getState();
  const resolved = resolveTheme(mode);
  const el = document.documentElement;

  for (const name of appliedTokens) el.style.removeProperty(name);
  appliedTokens = [];
  el.classList.remove(...THEME_CLASSES);

  if (resolved !== "omarchy") {
    if (resolved !== "dark") el.classList.add(resolved);
    return;
  }

  if (omarchy === null) {
    // Covers a persisted mode: "omarchy" booting on a machine where Omarchy
    // is gone, and the split-second before the first IPC round-trip lands on
    // a truly first launch.
    const fallback = prefersDark() ? "dark" : "light";
    if (fallback !== "dark") el.classList.add(fallback);
    return;
  }

  // The base class supplies exactly one token: --color-bad, which core
  // deliberately does not derive (Omarchy's `red` is not required to be red —
  // vantablack's is #a4a4a4). All 30 others are overwritten inline immediately
  // below, so the class contributes nothing else.
  //
  // Picking `light` for a light Omarchy theme rather than always falling to the
  // bare @theme is a legibility fix, not a second colour decision: measured
  // against all 22 built-in themes, always using @theme's #ef5350 bottoms out at
  // 3.08:1 on catppuccin-latte's near-white surface — below the 4.5:1 body-text
  // bar. Deferring to html.light's own #b91c1c on the five light themes lifts the
  // worst case across all 22 to 4.88:1 (everforest). It is still "the app's
  // default red", just the one the app already uses on a light surface.
  // `html.light` also carries `color-scheme: light` (index.css:228), which is
  // exactly what WebKitGTK needs for a light palette, so no inline colorScheme
  // write is required.
  if (omarchy.mode === "light") el.classList.add("light");
  for (const { name, value } of omarchy.tokens) {
    el.style.setProperty(name, value);
    appliedTokens.push(name);
  }
}

/** Fetch and store the active Omarchy palette. The two no-palette answers are
 *  handled differently — see the branches below. */
export async function refreshOmarchyTheme(): Promise<void> {
  const { installed, theme } = await commands.getOmarchyTheme();
  const { explicit, mode, omarchy, setOmarchy } = useThemeStore.getState();

  if (!installed) {
    // The one case that must clear. Omarchy is gone (or was never here), so a
    // persisted palette is stale for good — leaving it would repaint a dead
    // theme on every relaunch and keep the picker labelling it live instead of
    // "not detected" (Settings.tsx:132). This cannot be the theme-switch race
    // below: `installed` is a theme.name probe, and that file is only ever
    // truncated in place, never removed (see omarchy::status).
    if (omarchy !== null) setOmarchy(null);
    // Walk back the auto-select in kind. It sets `mode` without setting
    // `explicit` (see below), so this only undoes the machine's own choice —
    // a deliberate "Omarchy" pick survives, and re-installing re-selects.
    if (mode === "omarchy" && !explicit) useThemeStore.setState({ mode: "system" });
    return;
  }

  if (theme === null) {
    // Installed but no readable palette, which never clears a stored one.
    // Omarchy replaces the theme directory with `rm -rf` + `mv`
    // (omarchy-theme-set:164-165), so a poll tick landing inside that window
    // reads null for one cycle; dropping the user's theme for ~2s every time
    // they switch is worse than the gap this leaves. The accepted gap: a
    // permanently unreadable colors.toml keeps painting the last good palette,
    // logged core-side as a warning and invisible here.
    return;
  }

  setOmarchy(theme);
  // Neither guard is redundant: !explicit blocks overriding a deliberate
  // System, and mode === "system" blocks the (impossible with setMode, but
  // cheap to defend) case of a non-explicit non-System mode. This is not a
  // user choice, so it skips setMode deliberately and leaves `explicit`
  // untouched — a later manual pick still flips it and locks the choice in.
  if (!explicit && mode === "system") useThemeStore.setState({ mode: "omarchy" });
}

/** Wire reactivity: re-apply on selection change and (when on "system") on OS
 *  appearance change. Called once at startup from main.tsx. */
export function initTheme(): void {
  applyTheme();
  useThemeStore.subscribe(applyTheme);
  if (typeof window !== "undefined" && typeof window.matchMedia === "function") {
    window
      .matchMedia("(prefers-color-scheme: dark)")
      .addEventListener("change", () => {
        if (useThemeStore.getState().mode === "system") applyTheme();
      });
  }
  // Module-scope IPC at startup already has precedent (main.tsx:38-40 does
  // the same for listRunningScripts). Fire-and-forget: refreshOmarchyTheme()
  // decides on its own whether to touch the store, and applyTheme
  // (subscribed above) re-runs when it does.
  void refreshOmarchyTheme();
}
