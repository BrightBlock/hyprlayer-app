import { useEffect, useRef } from "react";

import { createTerminalEngine } from "./terminalEngine";
import { useTerminalFontSize } from "./terminalFontSize";
import { xtermThemeFromCss } from "./terminalTheme";

/**
 * Read-only render of a script's captured last-run output (terminal text incl.
 * ANSI). Reuses the same xterm engine as the live views, but just writes the
 * stored text once — no PTY, no input.
 */
export function ScriptOutputView({ output }: { output: string }) {
  const hostRef = useRef<HTMLDivElement>(null);
  const fontSize = useTerminalFontSize((s) => s.fontSize);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const engine = createTerminalEngine({ fontSize, theme: xtermThemeFromCss() });
    engine.mount(host);
    engine.fit();
    // xterm needs CRLF; normalize bare \n (captured Exec output) without doubling
    // existing CRLFs (captured PTY scrollback).
    engine.write(output.replace(/\r?\n/g, "\r\n"));

    let raf = 0;
    const ro = new ResizeObserver(() => {
      if (raf) return;
      raf = requestAnimationFrame(() => {
        raf = 0;
        if (host.clientWidth && host.clientHeight) engine.fit();
      });
    });
    ro.observe(host);

    return () => {
      if (raf) cancelAnimationFrame(raf);
      ro.disconnect();
      engine.dispose();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [output]);

  return (
    <div className="relative min-h-0 min-w-0 flex-1 bg-[#070707]">
      <div ref={hostRef} className="absolute inset-2 overflow-hidden" />
    </div>
  );
}
