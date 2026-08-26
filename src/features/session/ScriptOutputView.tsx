import { useEffect, useRef } from "react";

import { attachTerminalClipboard, useTerminalThemeSync } from "@/lib/terminal/terminalChrome";
import { createTerminalEngine, type TerminalEngine } from "@/lib/terminal/terminalEngine";
import { useTerminalFontSize } from "@/lib/terminal/terminalFontSize";
import { xtermThemeFromCss } from "@/lib/terminal/terminalTheme";

export function ScriptOutputView({ output }: { output: string }) {
  const hostRef = useRef<HTMLDivElement>(null);
  const fontSize = useTerminalFontSize((s) => s.fontSize);
  const engineRef = useRef<TerminalEngine | null>(null);
  useTerminalThemeSync(engineRef);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const engine = createTerminalEngine({ fontSize, theme: xtermThemeFromCss() });
    engineRef.current = engine;
    engine.mount(host);
    attachTerminalClipboard(engine);
    engine.fit();
    engine.write(output.replace(/\r?\n/g, "\r\n") + "\x1b[?25l");

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
      engineRef.current = null;
      engine.dispose();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [output]);

  return (
    <div className="relative min-h-0 min-w-0 flex-1 bg-surface">
      <div ref={hostRef} className="absolute inset-2 overflow-hidden" />
    </div>
  );
}
