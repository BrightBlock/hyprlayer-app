
import { useEffect, type RefObject } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";

import { monoFontStack, useAppearance } from "@/lib/appearance";
import { commands } from "@/lib/bindings";
import { useCommand } from "@/lib/keybindings/registry";
import { IS_MACOS } from "@/lib/platform";
import { useThemeStore } from "@/lib/theme";
import type { TerminalEngine, TerminalGrid } from "./terminalEngine";
import { useTerminalFontSize } from "./terminalFontSize";
import { isDarkBackground, terminalColors, xtermThemeFromCss } from "./terminalTheme";

let sentDark: boolean | undefined;
let sentColors: string | undefined;

function sendColorScheme(): void {
  const theme = xtermThemeFromCss();
  const dark = isDarkBackground(theme.background);
  if (dark !== sentDark) {
    sentDark = dark;
    void commands.setTerminalColorScheme(dark).catch(() => {});
  }
  const colors = terminalColors(theme);
  const key = JSON.stringify(colors);
  if (key === sentColors) return;
  sentColors = key;
  void commands.setTerminalTheme(colors).catch(() => {});
}

/** `COLORFGBG` for every pty spawned from here on, following the theme. Call after `initTheme`. */
export function syncTerminalColorScheme(): void {
  if (!isTauri()) return;
  sendColorScheme();
  useThemeStore.subscribe(sendColorScheme);
  window.matchMedia?.("(prefers-color-scheme: dark)").addEventListener("change", sendColorScheme);
}

/** `sizes`, when given, says whether this view may resize the pty; unset, it always may. */
export function useTerminalZoom(
  engineRef: RefObject<TerminalEngine | null>,
  idRef: RefObject<string | undefined>,
  sizes?: RefObject<boolean>,
): number {
  const fontSize = useTerminalFontSize((s) => s.fontSize);
  const zoomIn = useTerminalFontSize((s) => s.zoomIn);
  const zoomOut = useTerminalFontSize((s) => s.zoomOut);
  const zoomReset = useTerminalFontSize((s) => s.reset);

  useCommand("terminal.zoomIn", zoomIn);
  useCommand("terminal.zoomOut", zoomOut);
  useCommand("terminal.zoomReset", zoomReset);

  useTerminalFontSync(engineRef, idRef, sizes);

  return fontSize;
}

export function useTerminalFontSync(
  engineRef: RefObject<TerminalEngine | null>,
  idRef?: RefObject<string | undefined>,
  sizes?: RefObject<boolean>,
): void {
  useEffect(() => {
    const resize = (g: TerminalGrid) => {
      const id = idRef?.current;
      if (sizes && !sizes.current) return;
      if (id && g.cols > 0 && g.rows > 0) void commands.terminalResize(id, g.cols, g.rows);
    };
    const unsubFamily = useAppearance.subscribe((s, prev) => {
      const engine = engineRef.current;
      if (engine && s.monoFont !== prev.monoFont)
        void engine.setFontFamily(monoFontStack(s.monoFont)).then(resize);
    });
    const unsubSize = useTerminalFontSize.subscribe((s, prev) => {
      const engine = engineRef.current;
      if (engine && s.fontSize !== prev.fontSize) resize(engine.setFontSize(s.fontSize));
    });
    return () => {
      unsubFamily();
      unsubSize();
    };
  }, [engineRef, idRef, sizes]);
}

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

// WebKit enables the menu's Copy only for a DOM selection, and xterm keeps its own, so ⌘C is ours too.
function clipboardKey(e: KeyboardEvent): "c" | "v" | null {
  const chord = IS_MACOS
    ? e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey
    : e.ctrlKey && e.shiftKey && !e.altKey && !e.metaKey;
  const key = e.key.toLowerCase();
  return chord && (key === "c" || key === "v") ? key : null;
}

export function attachTerminalClipboard(engine: TerminalEngine): void {
  engine.interceptKeys((e) => {
    if (e.type !== "keydown") return true;
    // xterm maps ctrl+_ to ^_ and stops propagation, which would swallow terminal.zoomOut.
    if (!IS_MACOS && e.ctrlKey && e.shiftKey && !e.altKey && !e.metaKey && e.key === "_") {
      return false;
    }
    const key = clipboardKey(e);
    if (!key) return true;
    e.preventDefault();
    e.stopPropagation();
    if (key === "c") {
      const text = engine.selectionText();
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
