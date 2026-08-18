/**
 * Embedded xterm panel that runs a `hyprlayer` subcommand (e.g. `["ai",
 * "configure"]`) in a real PTY. Modelled on TerminalTab.tsx but spawned via
 * `spawnCliTerminal` instead of the workstream-coupled `terminalOpen`.
 */
import { useEffect, useRef } from "react";
import { Channel } from "@tauri-apps/api/core";

import { commands, type TerminalEvent } from "@/lib/bindings";
import { unwrap } from "@/lib/ipc";
import { useThemeStore } from "@/lib/theme";
import { attachTerminalClipboard } from "@/lib/terminal/terminalChrome";
import { b64ToBytes, strToB64 } from "@/lib/terminal/terminalCodec";
import { createTerminalEngine, type TerminalEngine } from "@/lib/terminal/terminalEngine";
import { useTerminalFontSize } from "@/lib/terminal/terminalFontSize";
import { xtermThemeFromCss } from "@/lib/terminal/terminalTheme";

type Props = {
  /** Arguments passed to `hyprlayer`, e.g. ["ai", "configure"]. Ignored when
   *  `shellCommand` is set. */
  args?: string[];
  /** Run a single command line through the user's login shell instead of the
   *  hyprlayer CLI, e.g. `"claude auth login"` or an installer one-liner. */
  shellCommand?: string;
  /** Called once when the process exits. `code` is null on SIGKILL. */
  onExit?: (code: number | null) => void;
  className?: string;
};

export function SetupTerminal({ args, shellCommand, onExit, className = "" }: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const engineRef = useRef<TerminalEngine | null>(null);
  const idRef = useRef<string | undefined>(undefined);
  const appliedFontSizeRef = useRef<number>(0);

  const fontSize = useTerminalFontSize((s) => s.fontSize);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const engine = createTerminalEngine({ fontSize, theme: xtermThemeFromCss() });
    engine.mount(host);
    // Ctrl+Shift+C / Ctrl+Shift+V off macOS, the emulator has no copy
    // path of its own there (see attachTerminalClipboard).
    attachTerminalClipboard(engine);
    engine.focus();
    engineRef.current = engine;
    appliedFontSizeRef.current = fontSize;
    let lastGrid = engine.fit();

    const channel = new Channel<TerminalEvent>();
    let disposed = false;

    channel.onmessage = (ev) => {
      if (ev.kind === "output") {
        engine.write(b64ToBytes(ev.data));
      } else if (ev.kind === "exited") {
        engine.write("\r\n\x1b[2m[process exited]\x1b[0m\r\n");
        // Only surface a GENUINE exit. Teardown (our cleanup's `terminalClose`,
        // or a StrictMode mount→unmount→mount whose in-flight PTY we kill) also
        // fires `exited`; calling back then would bounce the parent out of the
        // configure step on a phantom completion.
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
      // Reconcile PTY size now that the id is live.
      lastGrid = engine.fit();
      void commands.terminalResize(termId, lastGrid.cols, lastGrid.rows);
    })();

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
    // args is intentionally read once at mount (same semantics as workstreamId
    // in TerminalTab): re-running with new args would tear down the live PTY.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Apply zoom in place.
  useEffect(() => {
    const engine = engineRef.current;
    if (!engine || appliedFontSizeRef.current === fontSize) return;
    appliedFontSizeRef.current = fontSize;
    const g = engine.setFontSize(fontSize);
    const id = idRef.current;
    if (id) void commands.terminalResize(id, g.cols, g.rows);
  }, [fontSize]);

  // Recolor live when the theme changes.
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
  }, []);

  return (
    <div className={`relative min-h-0 min-w-0 flex-1 bg-surface ${className}`}>
      <div ref={hostRef} className="absolute inset-2 overflow-hidden" />
    </div>
  );
}
