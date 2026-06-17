/**
 * Color themes, ported from termic (github.com/simion/termic).
 *
 * A theme is just a set of CSS variables (defined in index.css under either
 * `@theme` for the default Dark+ palette or an `html.<name>` block). This
 * module owns the *choice* of theme: it persists the user's selection and
 * toggles the matching class on <html> so the variables — and therefore every
 * `bg-surface` / `text-ink` / `border-edge` utility — re-resolve.
 */

import { create } from "zustand";
import { persist } from "zustand/middleware";

/** "system" follows the OS appearance; the rest map to a concrete palette. */
export type ThemeMode = "system" | "dark" | "light" | "claude" | "cobalt" | "matrix" | "solarized";

/** What `system` resolves to, and the set of palettes index.css defines. */
export type ResolvedTheme = "dark" | "light" | "claude" | "cobalt" | "matrix" | "solarized";

/** Picker order + labels. Mirrors termic's theme menu. */
export const THEMES: ReadonlyArray<{ id: ThemeMode; label: string }> = [
  { id: "system", label: "System" },
  { id: "light", label: "Light" },
  { id: "claude", label: "Claude" },
  { id: "dark", label: "Dark+" },
  { id: "solarized", label: "Solarized Dark" },
  { id: "cobalt", label: "Cobalt" },
  { id: "matrix", label: "Matrix" },
];

// THEMES is the single source of truth; the two lists below derive from it so
// adding a theme is a one-line change.
const VALID_MODES: ReadonlyArray<ThemeMode> = THEMES.map((t) => t.id);
/** Palettes that need a class on <html>. "system" resolves elsewhere and "dark"
 *  is the bare `@theme` default (no class) — selecting it just clears the rest. */
const THEME_CLASSES = VALID_MODES.filter(
  (id): id is ResolvedTheme => id !== "system" && id !== "dark",
);

const prefersDark = () =>
  typeof window !== "undefined" &&
  typeof window.matchMedia === "function" &&
  window.matchMedia("(prefers-color-scheme: dark)").matches;

/** Resolve the user's choice to a concrete palette. `system` checks the OS. */
export function resolveTheme(mode: ThemeMode): ResolvedTheme {
  if (mode === "system") return prefersDark() ? "dark" : "light";
  return mode;
}

type ThemeStore = {
  /** The user's selection (may be "system"). Defaults to the Claude palette;
   *  a persisted choice (below) overrides it for users who've picked another. */
  mode: ThemeMode;
  setMode: (mode: ThemeMode) => void;
};

export const useThemeStore = create<ThemeStore>()(
  persist(
    (set) => ({
      mode: "claude",
      setMode: (mode) => set({ mode }),
    }),
    {
      name: "hyprlayer:theme",
      version: 1,
      // Drop unknown ids (e.g. a theme removed in a later version) so we fall
      // back to the default rather than landing on the bare @theme silently.
      merge: (persisted, current) => {
        const raw = (persisted as Partial<ThemeStore> | undefined)?.mode;
        const mode = VALID_MODES.includes(raw as ThemeMode) ? (raw as ThemeMode) : current.mode;
        return { ...current, mode };
      },
    },
  ),
);

/** Toggle the resolved palette's class on <html>. Safe to call repeatedly. */
export function applyTheme(): void {
  if (typeof document === "undefined") return;
  const resolved = resolveTheme(useThemeStore.getState().mode);
  const el = document.documentElement;
  el.classList.remove(...THEME_CLASSES);
  if (resolved !== "dark") el.classList.add(resolved);
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
}
