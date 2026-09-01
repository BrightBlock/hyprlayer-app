import { useEffect, useRef } from "react";
import { Channel } from "@tauri-apps/api/core";

import { commands, type ScriptKind, type TerminalEvent } from "@/lib/bindings";
import { releaseChannels } from "@/lib/terminal/channelRelease";
import { errorMessage, errorRemedy, unwrap } from "@/lib/ipc";
import { attachTerminalClipboard, useTerminalThemeSync } from "@/lib/terminal/terminalChrome";
import { b64ToBytes, strToB64 } from "@/lib/terminal/terminalCodec";
import { createTerminalEngine, type TerminalEngine } from "@/lib/terminal/terminalEngine";
import { useTerminalFontSize } from "@/lib/terminal/terminalFontSize";
import { xtermThemeFromCss } from "@/lib/terminal/terminalTheme";

type Props = {
  workstreamId: string;
  kind: ScriptKind;
  attachId: string | null;
  gen: number;
  clearNonce: number;
  onStarted?: (terminalId: string) => void;
  onExit?: () => void;
  onDead?: () => void;
};

export function ScriptRunView({
  workstreamId,
  kind,
  attachId,
  gen,
  clearNonce,
  onStarted,
  onExit,
  onDead,
}: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const fontSize = useTerminalFontSize((s) => s.fontSize);
  const engineRef = useRef<TerminalEngine | null>(null);
  useTerminalThemeSync(engineRef);
  const attachRef = useRef(attachId);
  attachRef.current = attachId;
  const onStartedRef = useRef(onStarted);
  onStartedRef.current = onStarted;
  const onExitRef = useRef(onExit);
  onExitRef.current = onExit;
  const onDeadRef = useRef(onDead);
  onDeadRef.current = onDead;

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const engine = createTerminalEngine({ fontSize, theme: xtermThemeFromCss() });
    engineRef.current = engine;
    engine.mount(host);
    attachTerminalClipboard(engine);
    let lastGrid = engine.fit();
    engine.focus();

    let disposed = false;
    let id: string | undefined = attachRef.current ?? undefined;
    const reattaching = id != null;

    const created: Channel<TerminalEvent>[] = [];
    const newChannel = (): Channel<TerminalEvent> => {
      const channel = new Channel<TerminalEvent>();
      created.push(channel);
      channel.onmessage = (ev) => {
        if (ev.kind === "output") engine.write(b64ToBytes(ev.data));
        else if (ev.kind === "exited") {
          engine.write("\r\n\x1b[2m[process exited]\x1b[0m\r\n\x1b[?25l");
          onExitRef.current?.();
        }
      };
      return channel;
    };

    void (async () => {
      if (id) {
        const ok = await unwrap(commands.terminalAttach(id, newChannel()));
        if (!ok) id = undefined;
      }
      if (reattaching && !id) {
        if (!disposed) onDeadRef.current?.();
        return;
      }
      if (!id && !disposed) {
        await new Promise<void>((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
        );
        if (disposed) return;
        lastGrid = engine.fit();
        try {
          const opened = await unwrap(
            commands.scriptRun(workstreamId, kind, lastGrid.cols, lastGrid.rows, newChannel()),
          );
          if (disposed) return;
          id = opened;
          onStartedRef.current?.(opened);
        } catch (e) {
          const remedy = errorRemedy(e);
          const crlf = (s: string): string => s.replace(/\r?\n/g, "\r\n");
          engine.write(
            `\r\n\x1b[31m${crlf(errorMessage(e))}\x1b[0m\r\n` +
              (remedy ? `\x1b[2m${crlf(remedy)}\x1b[0m\r\n` : "") +
              "\x1b[?25l",
          );
          onExitRef.current?.();
          return;
        }
      }
      if (id && !disposed) {
        lastGrid = engine.fit();
        void commands.terminalResize(id, lastGrid.cols, lastGrid.rows);
      }
    })();

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
    const ro = new ResizeObserver(() => {
      if (!raf) raf = requestAnimationFrame(doFit);
    });
    ro.observe(host);

    return () => {
      disposed = true;
      if (raf) cancelAnimationFrame(raf);
      unsubInput();
      ro.disconnect();
      engineRef.current = null;
      engine.dispose();
      releaseChannels(created);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workstreamId, gen]);

  const appliedClear = useRef(clearNonce);
  useEffect(() => {
    if (clearNonce === appliedClear.current) return;
    appliedClear.current = clearNonce;
    engineRef.current?.reset();
  }, [clearNonce]);

  return (
    <div className="relative min-h-0 min-w-0 flex-1 bg-surface">
      <div ref={hostRef} className="absolute inset-2 overflow-hidden" />
    </div>
  );
}
