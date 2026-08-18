//! Shared terminal "chrome" — font-size zoom, live theme recolor, and the
//! clipboard chords — used by every mounted terminal (TerminalTab, the
//! multiplexer AgentPtyTab, the script views, onboarding). These act purely on
//! the TerminalEngine abstraction (setFontSize / setTheme / selectionText) and
//! never touch the PTY spawn/attach/resize data flow, so every terminal shares
//! one implementation instead of copying the effects.

import { useEffect, type RefObject } from "react";
import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";

import { commands } from "@/lib/bindings";
import { useCommand } from "@/lib/keybindings/registry";
import { IS_MACOS } from "@/lib/platform";
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

/**
 * Give a terminal the clipboard chords its emulator doesn't provide off macOS:
 * Ctrl+Shift+C copies the selection, Ctrl+Shift+V pastes — the convention every
 * Linux/Windows terminal follows, and the only way to move text between a
 * terminal and the composer.
 *
 * Without this there is no copy path at all off macOS. xterm rewrites
 * Ctrl+letter to a control byte and preventDefaults it (Ctrl+C is SIGINT,
 * Ctrl+V is ^V), so the webview's native copy never fires and xterm's own
 * `copy` handler — which would have put the selection on the clipboard — is
 * never reached. macOS escapes this because ⌘C/⌘V match none of xterm's key
 * branches and fall through to the webview; that path is left alone, so this is
 * a no-op there.
 *
 * Clipboard I/O goes through Tauri rather than `navigator.clipboard`: the async
 * clipboard is exactly the surface WebKitGTK has already been caught lying
 * about (see the image-paste fallback in src-tauri/src/lib.rs). Needs
 * `clipboard-manager:allow-read-text` / `allow-write-text` in the capability
 * file.
 *
 * Call once, right after `engine.mount(host)` — an engine holds a single
 * filter, and it is torn down with the engine.
 */
export function attachTerminalClipboard(engine: TerminalEngine): void {
  if (IS_MACOS) return;
  engine.interceptKeys((e) => {
    // keypress consults the same filter; only judge the keydown.
    if (e.type !== "keydown") return true;
    if (!e.ctrlKey || !e.shiftKey || e.altKey || e.metaKey) return true;
    const key = e.key.toLowerCase();
    if (key !== "c" && key !== "v") return true;
    // Ours: keep it from the emulator, from the webview, and from the
    // window-level keybinding dispatcher — `mod+shift+v` is Toggle Markdown
    // Preview, which is live whenever a markdown file viewer sits beside the
    // terminal. Returning false stops xterm but does NOT preventDefault, so
    // both calls are on us.
    e.preventDefault();
    e.stopPropagation();
    if (key === "c") {
      const text = engine.selectionText();
      // Nothing selected: swallow the chord rather than letting it through as a
      // control byte. Copying "" would also blank whatever is on the clipboard.
      if (text) void writeText(text).catch(() => {});
    } else {
      void readText().then(
        (text) => {
          if (text) engine.paste(text);
        },
        () => {},
      );
    }
    return false;
  });
}
