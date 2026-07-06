//! Shared terminal "chrome" effects — font-size zoom and live theme recolor —
//! used by every mounted terminal (TerminalTab and the multiplexer AgentPtyTab).
//! These act purely on the TerminalEngine abstraction (setFontSize / setTheme)
//! and never touch the PTY spawn/attach/resize data flow, so both terminals
//! share one implementation instead of copying the effects.

import { useEffect, type RefObject } from "react";

import { commands } from "@/lib/bindings";
import { useCommand } from "@/lib/keybindings/registry";
import { useThemeStore } from "@/lib/theme";
import type { TerminalEngine } from "./terminalEngine";
import { useTerminalFontSize } from "./terminalFontSize";
import { xtermThemeFromCss } from "./terminalTheme";

/**
 * Register the `terminal.zoom*` keyboard commands and apply zoom in place: set
 * the engine's font, refit to the new cell size, and tell the PTY its new grid.
 * Returns the live font size so the caller can build its engine at that size.
 *
 * Registering the commands here is the context gate — ⌘+/⌘-/⌘0 are inert when
 * no terminal is on screen. `appliedFontSizeRef` (which the caller seeds to the
 * current size right after creating the engine at it) skips the no-op first run.
 */
export function useTerminalZoom(
  engineRef: RefObject<TerminalEngine | null>,
  idRef: RefObject<string | undefined>,
  appliedFontSizeRef: RefObject<number>,
): number {
  const fontSize = useTerminalFontSize((s) => s.fontSize);
  const zoomIn = useTerminalFontSize((s) => s.zoomIn);
  const zoomOut = useTerminalFontSize((s) => s.zoomOut);
  const zoomReset = useTerminalFontSize((s) => s.reset);

  useCommand("terminal.zoomIn", zoomIn);
  useCommand("terminal.zoomOut", zoomOut);
  useCommand("terminal.zoomReset", zoomReset);

  useEffect(() => {
    const engine = engineRef.current;
    if (!engine || appliedFontSizeRef.current === fontSize) return;
    appliedFontSizeRef.current = fontSize;
    const g = engine.setFontSize(fontSize);
    if (idRef.current) void commands.terminalResize(idRef.current, g.cols, g.rows);
  }, [fontSize, engineRef, idRef, appliedFontSizeRef]);

  return fontSize;
}

/**
 * Recolor the engine live when the theme changes. Re-read on the next frame so
 * the <html> theme class (toggled by the theme store's own subscriber) has
 * applied before sampling the CSS variables; also covers "system" → OS
 * appearance.
 */
export function useTerminalThemeSync(engineRef: RefObject<TerminalEngine | null>): void {
  useEffect(() => {
    const recolor = () => engineRef.current?.setTheme(xtermThemeFromCss());
    const schedule = () => requestAnimationFrame(recolor);
    const unsub = useThemeStore.subscribe(schedule);
    const mq = window.matchMedia?.("(prefers-color-scheme: dark)");
    const onMq = () => {
      if (useThemeStore.getState().mode === "system") schedule();
    };
    mq?.addEventListener("change", onMq);
    return () => {
      unsub();
      mq?.removeEventListener("change", onMq);
    };
  }, [engineRef]);
}
