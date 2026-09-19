import { useEffect, useRef } from "react";
import { Channel } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { commands, type TerminalEvent } from "@/lib/bindings";
import { releaseChannels } from "@/lib/terminal/channelRelease";
import { errorMessage, unwrap } from "@/lib/ipc";
import { log } from "@/lib/log";
import { b64ToBytes, strToB64 } from "@/lib/terminal/terminalCodec";
import {
  attachTerminalClipboard,
  useTerminalThemeSync,
  useTerminalZoom,
} from "@/lib/terminal/terminalChrome";
import { createTerminalEngine, type TerminalEngine } from "@/lib/terminal/terminalEngine";
import { xtermThemeFromCss } from "@/lib/terminal/terminalTheme";

export function TerminalTab({
  workstreamId,
  shellId,
  terminalId,
  autoFocus,
  onTerminalId,
}: {
  workstreamId: string;
  shellId: number;
  terminalId?: string;
  autoFocus?: boolean;
  onTerminalId: (id: string) => void;
}) {
  const hostRef = useRef<HTMLDivElement>(null);
  const engineRef = useRef<TerminalEngine | null>(null);
  const idRef = useRef<string | undefined>(terminalId);
  const appliedFontSizeRef = useRef<number>(0);

  const fontSize = useTerminalZoom(engineRef, idRef, appliedFontSizeRef);
  useTerminalThemeSync(engineRef);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const engine = createTerminalEngine({ fontSize, theme: xtermThemeFromCss() });
    engine.mount(host);
    attachTerminalClipboard(engine);
    engineRef.current = engine;
    appliedFontSizeRef.current = fontSize;
    let lastGrid = engine.fit();

    if (autoFocus) engine.focus();

    // Tauri Channels are single-use, including failed attach attempts.
    const created: Channel<TerminalEvent>[] = [];
    const newChannel = (): Channel<TerminalEvent> => {
      const channel = new Channel<TerminalEvent>();
      created.push(channel);
      channel.onmessage = (ev) => {
        if (ev.kind === "output") engine.write(b64ToBytes(ev.data));
        else if (ev.kind === "exited")
          engine.write("\r\n\x1b[2m[process exited]\x1b[0m\r\n");
      };
      return channel;
    };

    let disposed = false;
    let id = terminalId;
    idRef.current = id;
    void (async () => {
      if (id) {
        const ok = await unwrap(commands.terminalAttach(id, newChannel()));
        if (!ok) {
          id = undefined;
          idRef.current = undefined;
        }
      }
      if (!id && !disposed) {
        await new Promise<void>((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
        );
        if (disposed) return;
        lastGrid = engine.fit();
        const opened = await unwrap(
          commands.terminalOpen(workstreamId, lastGrid.cols, lastGrid.rows, newChannel()),
        );
        if (disposed) {
          void commands.terminalClose(opened);
          return;
        }
        id = opened;
        idRef.current = id;
        onTerminalId(id);
      }
      if (id && !disposed) {
        lastGrid = engine.fit();
        void commands.terminalResize(id, lastGrid.cols, lastGrid.rows);
      }
    })().catch((e) => {
      // Otherwise the pane stays blank with no reason anywhere.
      log.error("terminal", "opening the terminal failed", e);
      if (!disposed) engine.write(`\r\n\x1b[31m${errorMessage(e)}\x1b[0m\r\n`);
    });

    const unsubInput = engine.onInput((data) => {
      if (id) void commands.terminalWrite(id, strToB64(data));
    });

    let raf = 0;
    const doFit = () => {
      raf = 0;
      if (disposed || host.clientWidth === 0 || host.clientHeight === 0) return;
      const next = engine.fit();
      if (next.cols === lastGrid.cols && next.rows === lastGrid.rows) return;
      lastGrid = next;
      if (id) void commands.terminalResize(id, next.cols, next.rows);
    };
    const scheduleFit = () => {
      if (!raf) raf = requestAnimationFrame(doFit);
    };
    const ro = new ResizeObserver(scheduleFit);
    ro.observe(host);

    let unlistenFocus: (() => void) | undefined;
    let focusCancelled = false;
    void getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => {
        if (!focused || disposed) return;
        scheduleFit();
        engine.refresh();
      })
      .then((fn) => {
        if (focusCancelled) fn();
        else unlistenFocus = fn;
      });

    return () => {
      disposed = true;
      focusCancelled = true;
      unlistenFocus?.();
      if (raf) cancelAnimationFrame(raf);
      unsubInput();
      ro.disconnect();
      engine.dispose();
      engineRef.current = null;
      releaseChannels(created);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workstreamId, shellId]);

  return (
    <div className="relative min-h-0 min-w-0 flex-1 bg-surface">
      <div ref={hostRef} className="absolute inset-2 overflow-hidden" />
    </div>
  );
}
