
import { create } from "zustand";
import { persist } from "zustand/middleware";

import { commands, type OmarchyTheme } from "./bindings";

export type ThemeMode =
  | "system"
  | "omarchy"
  | "dark"
  | "light"
  | "claude"
  | "cobalt"
  | "matrix"
  | "solarized";

export type ResolvedTheme =
  | "omarchy"
  | "dark"
  | "light"
  | "claude"
  | "cobalt"
  | "matrix"
  | "solarized";

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

const VALID_MODES: ReadonlyArray<ThemeMode> = THEMES.map((t) => t.id);
const THEME_CLASSES = VALID_MODES.filter(
  (id): id is ResolvedTheme => id !== "system" && id !== "dark" && id !== "omarchy",
);

const prefersDark = () =>
  typeof window !== "undefined" &&
  typeof window.matchMedia === "function" &&
  window.matchMedia("(prefers-color-scheme: dark)").matches;

export type AppearanceSegment = "light" | "dark" | "system";

/** The Light / Dark / Auto choice a mode shows as; named themes show none. */
export function appearanceSegment(mode: ThemeMode): AppearanceSegment | null {
  return mode === "light" || mode === "dark" || mode === "system" ? mode : null;
}

export function resolveTheme(mode: ThemeMode): ResolvedTheme {
  if (mode === "system") return prefersDark() ? "dark" : "light";
  return mode;
}

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
  mode: ThemeMode;
  explicit: boolean;
  omarchy: OmarchyTheme | null;
  setMode: (mode: ThemeMode) => void;
  setOmarchy: (theme: OmarchyTheme | null) => void;
};

export const useThemeStore = create<ThemeStore>()(
  persist(
    (set) => ({
      mode: "system",
      explicit: false,
      omarchy: null,
      setMode: (mode) => set({ mode, explicit: true }),
      setOmarchy: (theme) => set({ omarchy: theme }),
    }),
    {
      name: "hyprlayer:theme",
      version: 2,
      migrate: (persisted, _from) => ({
        ...(persisted as object),
        explicit: true,
      }),
      merge: (persisted, current) => {
        const p = persisted as Partial<ThemeStore> | undefined;
        const mode = VALID_MODES.includes(p?.mode as ThemeMode)
          ? (p!.mode as ThemeMode)
          : current.mode;
        return {
          ...current,
          mode,
          explicit: p === undefined ? current.explicit : p.explicit !== false,
          omarchy: isOmarchyTheme(p?.omarchy) ? p!.omarchy : null,
        };
      },
    },
  ),
);

let appliedTokens: string[] = [];

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
    const fallback = prefersDark() ? "dark" : "light";
    if (fallback !== "dark") el.classList.add(fallback);
    return;
  }

  if (omarchy.mode === "light") el.classList.add("light");
  for (const { name, value } of omarchy.tokens) {
    el.style.setProperty(name, value);
    appliedTokens.push(name);
  }
}

export async function refreshOmarchyTheme(): Promise<void> {
  const { installed, theme } = await commands.getOmarchyTheme();
  const { explicit, mode, omarchy, setOmarchy } = useThemeStore.getState();

  if (!installed) {
    if (omarchy !== null) setOmarchy(null);
    if (mode === "omarchy" && !explicit) useThemeStore.setState({ mode: "system" });
    return;
  }

  if (theme === null) {
    return;
  }

  setOmarchy(theme);
  if (!explicit && mode === "system") useThemeStore.setState({ mode: "omarchy" });
}

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
  void refreshOmarchyTheme();
}
