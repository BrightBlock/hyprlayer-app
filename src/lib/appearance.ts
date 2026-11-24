
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { create } from "zustand";
import { persist } from "zustand/middleware";

import { commands } from "./bindings";
import { unwrap } from "./ipc";
import { useCommand } from "./keybindings/registry";
import { TRAFFIC_LIGHTS, titlebarInsetLights } from "./layout";
import { log } from "./log";
import { IS_MACOS } from "./platform";

// VS Code's model, zoom = 1.2^level, without its negative levels (they break UI alignment).
const ZOOM_BASE = 1.2;
const ZOOM_LEVELS = 8;
export const ZOOM_MIN = 1;
export const ZOOM_MAX = ZOOM_BASE ** ZOOM_LEVELS;
export const ZOOM_DEFAULT = 1;

const clampZoom = (z: number): number => Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, z));
const zoomLevel = (z: number): number => Math.log(z) / Math.log(ZOOM_BASE);
const atLevel = (level: number): number =>
  ZOOM_BASE ** Math.min(ZOOM_LEVELS, Math.max(0, level));
// The epsilon lets an older 0.1-step zoom (e.g. 1.3) snap onto the nearest level.
const nextZoom = (z: number): number => atLevel(Math.floor(zoomLevel(z) + 1e-6) + 1);
const prevZoom = (z: number): number => atLevel(Math.ceil(zoomLevel(z) - 1e-6) - 1);

export const TEXT_SIZES = [
  { id: "small", label: "Small", scale: 0.9 },
  { id: "default", label: "Default", scale: 1 },
  { id: "large", label: "Large", scale: 1.12 },
  { id: "larger", label: "Larger", scale: 1.25 },
] as const;
type TextSizeId = (typeof TEXT_SIZES)[number]["id"];

export const SANS_FONTS = [
  { id: "inter", label: "Inter", stack: '"Inter Variable", "Inter", system-ui, sans-serif' },
  { id: "geist", label: "Geist", stack: '"Geist Variable", system-ui, sans-serif' },
  {
    id: "ibm-plex-sans",
    label: "IBM Plex Sans",
    stack: '"IBM Plex Sans Variable", system-ui, sans-serif',
  },
] as const;
export const MONO_FONTS = [
  {
    id: "jetbrains-mono",
    label: "JetBrains Mono",
    stack: '"JetBrains Mono", ui-monospace, monospace',
  },
  { id: "geist-mono", label: "Geist Mono", stack: '"Geist Mono Variable", ui-monospace, monospace' },
  { id: "fira-code", label: "Fira Code", stack: '"Fira Code Variable", ui-monospace, monospace' },
] as const;
type SansFontId = (typeof SANS_FONTS)[number]["id"];
type MonoFontId = (typeof MONO_FONTS)[number]["id"];

const sansFontStack = (id: SansFontId): string =>
  (SANS_FONTS.find((f) => f.id === id) ?? SANS_FONTS[0]).stack;

export const monoFontStack = (id: MonoFontId = useAppearance.getState().monoFont): string =>
  (MONO_FONTS.find((f) => f.id === id) ?? MONO_FONTS[0]).stack;

type AppearanceStore = {
  zoom: number;
  zoomIn: () => void;
  zoomOut: () => void;
  resetZoom: () => void;
  textSize: TextSizeId;
  setTextSize: (id: TextSizeId) => void;
  sansFont: SansFontId;
  setSansFont: (id: SansFontId) => void;
  monoFont: MonoFontId;
  setMonoFont: (id: MonoFontId) => void;
};

export const useAppearance = create<AppearanceStore>()(
  persist(
    (set) => ({
      zoom: ZOOM_DEFAULT,
      zoomIn: () => set((s) => ({ zoom: nextZoom(s.zoom) })),
      zoomOut: () => set((s) => ({ zoom: prevZoom(s.zoom) })),
      resetZoom: () => set({ zoom: ZOOM_DEFAULT }),
      textSize: "default",
      setTextSize: (id) => set({ textSize: id }),
      sansFont: "inter",
      setSansFont: (id) => set({ sansFont: id }),
      monoFont: "jetbrains-mono",
      setMonoFont: (id) => set({ monoFont: id }),
    }),
    {
      name: "hyprlayer:appearance",
      version: 1,
      merge: (persisted, current) => {
        const p = persisted as Partial<AppearanceStore> | undefined;
        return {
          ...current,
          zoom:
            typeof p?.zoom === "number" && Number.isFinite(p.zoom)
              ? clampZoom(p.zoom)
              : current.zoom,
          textSize: TEXT_SIZES.some((t) => t.id === p?.textSize)
            ? p!.textSize!
            : current.textSize,
          sansFont: SANS_FONTS.some((f) => f.id === p?.sansFont)
            ? p!.sansFont!
            : current.sansFont,
          monoFont: MONO_FONTS.some((f) => f.id === p?.monoFont)
            ? p!.monoFont!
            : current.monoFont,
        };
      },
    },
  ),
);

function applyZoom(zoom: number): void {
  if (!isTauri()) return;
  void getCurrentWebview()
    .setZoom(zoom)
    .then(() => window.dispatchEvent(new Event("resize")))
    .catch((e) => log.warn("appearance", "could not set the app zoom", e));
  if (IS_MACOS) {
    void unwrap(
      commands.setTrafficLightInset(TRAFFIC_LIGHTS.x * zoom, TRAFFIC_LIGHTS.y * zoom),
    ).catch((e) => log.warn("appearance", "could not move the traffic lights", e));
  }
}

export function useTitlebarInsetLights(): number {
  return titlebarInsetLights(useAppearance((s) => s.zoom));
}

function applyStyle(s: AppearanceStore): void {
  const scale = TEXT_SIZES.find((t) => t.id === s.textSize)?.scale ?? 1;
  document.documentElement.style.setProperty("--text-scale", String(scale));
  document.documentElement.style.setProperty("--font-sans", sansFontStack(s.sansFont));
  document.documentElement.style.setProperty("--font-mono", monoFontStack(s.monoFont));
}

function remeasure(): void {
  requestAnimationFrame(() => window.dispatchEvent(new Event("resize")));
}

export function initAppearance(): void {
  const state = useAppearance.getState();
  applyZoom(state.zoom);
  applyStyle(state);
  void document.fonts.load(`12px ${monoFontStack(state.monoFont)}`).catch(() => undefined);
  useAppearance.subscribe((s, prev) => {
    if (s.zoom !== prev.zoom) applyZoom(s.zoom);
    applyStyle(s);
    if (s.textSize !== prev.textSize) remeasure();
    if (s.sansFont !== prev.sansFont)
      void document.fonts.load(`13px ${sansFontStack(s.sansFont)}`).then(remeasure, remeasure);
    if (s.monoFont !== prev.monoFont)
      void document.fonts.load(`13px ${monoFontStack(s.monoFont)}`).then(remeasure, remeasure);
  });
}

export function useAppZoomCommands(): void {
  const { zoomIn, zoomOut, resetZoom } = useAppearance.getState();
  useCommand("view.zoomIn", zoomIn);
  useCommand("view.zoomOut", zoomOut);
  useCommand("view.zoomReset", resetZoom);
}
