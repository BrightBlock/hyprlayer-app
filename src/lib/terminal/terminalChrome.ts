
import { useEffect, type RefObject } from "react";
import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";

import { commands } from "@/lib/bindings";
import { useCommand } from "@/lib/keybindings/registry";
import { IS_MACOS } from "@/lib/platform";
import { useThemeStore } from "@/lib/theme";
import type { TerminalEngine } from "./terminalEngine";
import { useTerminalFontSize } from "./terminalFontSize";
import { xtermThemeFromCss } from "./terminalTheme";

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

export function attachTerminalClipboard(engine: TerminalEngine): void {
  if (IS_MACOS) return;
  engine.interceptKeys((e) => {
    if (e.type !== "keydown") return true;
    if (!e.ctrlKey || !e.shiftKey || e.altKey || e.metaKey) return true;
    const key = e.key.toLowerCase();
    if (key !== "c" && key !== "v") return true;
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
