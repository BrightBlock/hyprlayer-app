import { useEffect, useRef } from "react";
import { Channel } from "@tauri-apps/api/core";

import { monoFontStack } from "@/lib/appearance";
import { commands, type TerminalEvent } from "@/lib/bindings";
import { errorMessage, unwrap } from "@/lib/ipc";
import { log } from "@/lib/log";
import {
  attachTerminalClipboard,
  useTerminalFontSync,
  useTerminalThemeSync,
} from "@/lib/terminal/terminalChrome";
import { b64ToBytes, strToB64 } from "@/lib/terminal/terminalCodec";
import { createTerminalEngine, type TerminalEngine } from "@/lib/terminal/terminalEngine";
import { useTerminalFontSize } from "@/lib/terminal/terminalFontSize";
import { xtermThemeFromCss } from "@/lib/terminal/terminalTheme";

type Props = {
  args?: string[];
  shellCommand?: string;
  onExit?: (code: number | null) => void;
  className?: string;
};

export function SetupTerminal({ args, shellCommand, onExit, className = "" }: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const engineRef = useRef<TerminalEngine | null>(null);
  const idRef = useRef<string | undefined>(undefined);

  const fontSize = useTerminalFontSize((s) => s.fontSize);
  useTerminalThemeSync(engineRef);
  useTerminalFontSync(engineRef, idRef);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const engine = createTerminalEngine({
      fontSize,
      fontFamily: monoFontStack(),
      theme: xtermThemeFromCss(),
    });
    engine.mount(host);
    attachTerminalClipboard(engine);
    engine.focus();
    engineRef.current = engine;
    let lastGrid = engine.fit();

    const channel = new Channel<TerminalEvent>();
    let disposed = false;

    channel.onmessage = (ev) => {
      if (ev.kind === "output") {
        engine.write(b64ToBytes(ev.data));
      } else if (ev.kind === "exited") {
        engine.write("\r\n\x1b[2m[process exited]\x1b[0m\r\n");
        if (!disposed) onExit?.(ev.code ?? null);
      }
    };

    void (async () => {
      const g = engine.grid();
      const termId = await unwrap(
        shellCommand !== undefined
          ? commands.spawnShellTerminal(shellCommand, g.cols, g.rows, channel)
          : commands.spawnCliTerminal(args ?? [], g.cols, g.rows, channel),
      );
      if (disposed) {
        void commands.terminalClose(termId);
        return;
      }
      idRef.current = termId;
      lastGrid = engine.fit();
      void commands.terminalResize(termId, lastGrid.cols, lastGrid.rows);
    })().catch((e) => {
      log.error("setup-terminal", "starting the sign-in terminal failed", e);
      if (!disposed) engine.write(`\r\n\x1b[31m${errorMessage(e)}\x1b[0m\r\n`);
    });

    const unsubInput = engine.onInput((data) => {
      const id = idRef.current;
      if (id) void commands.terminalWrite(id, strToB64(data));
    });

    let raf = 0;
    const doFit = () => {
      raf = 0;
      if (disposed || host.clientWidth === 0 || host.clientHeight === 0) return;
      const next = engine.fit();
      if (next.cols === lastGrid.cols && next.rows === lastGrid.rows) return;
      lastGrid = next;
      const id = idRef.current;
      if (id) void commands.terminalResize(id, next.cols, next.rows);
    };
    const scheduleFit = () => {
      if (!raf) raf = requestAnimationFrame(doFit);
    };
    const ro = new ResizeObserver(scheduleFit);
    ro.observe(host);

    return () => {
      disposed = true;
      if (raf) cancelAnimationFrame(raf);
      unsubInput();
      ro.disconnect();
      engine.dispose();
      engineRef.current = null;
      const id = idRef.current;
      if (id) void commands.terminalClose(id);
      idRef.current = undefined;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className={`relative min-h-0 min-w-0 flex-1 bg-surface ${className}`}>
      <div ref={hostRef} className="absolute inset-2 overflow-hidden" />
    </div>
  );
}
