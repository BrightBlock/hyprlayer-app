import { useEffect, useRef, useState } from "react";

import { TakeSizeButton } from "@/components/TakeSizeButton";
import { monoFontStack } from "@/lib/appearance";
import { commands, type ScriptKind } from "@/lib/bindings";
import { onServerReconnected } from "@/lib/events";
import { errorMessage, errorRemedy, unwrap } from "@/lib/ipc";
import { log } from "@/lib/log";
import {
  attachTerminalClipboard,
  useTerminalFontSync,
  useTerminalThemeSync,
} from "@/lib/terminal/terminalChrome";
import { TerminalFeed } from "@/lib/terminal/terminalFeed";
import { writeTerminal } from "@/lib/terminal/terminalInput";
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
  const idRef = useRef<string | undefined>(undefined);
  const [sizeOwner, setSizeOwner] = useState<boolean | null>(null);
  useTerminalThemeSync(engineRef);
  useTerminalFontSync(engineRef, idRef);
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

    const engine = createTerminalEngine({
      fontSize,
      fontFamily: monoFontStack(),
      theme: xtermThemeFromCss(),
    });
    engineRef.current = engine;
    engine.mount(host);
    attachTerminalClipboard(engine);
    let lastGrid = engine.fit();
    engine.focus();

    let disposed = false;
    let id: string | undefined = attachRef.current ?? undefined;
    idRef.current = id;
    const reattaching = id != null;

    // A reconnect resumes only a stream this mount opened, and not once its process ended.
    let streaming = false;
    let ended = false;
    const feed = new TerminalFeed(
      engine,
      () => {
        ended = true;
        engine.write("\r\n\x1b[2m[process exited]\x1b[0m\r\n\x1b[?25l");
        onExitRef.current?.();
      },
      undefined,
      setSizeOwner,
    );

    void (async () => {
      if (id) {
        const ok = await unwrap(commands.terminalAttach(id, null, feed.channel())).catch((e) => {
          log.warn("script-run", `reattaching terminal ${id} failed`, e);
          return false;
        });
        if (!ok) {
          id = undefined;
          idRef.current = undefined;
        }
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
            commands.scriptRun(workstreamId, kind, lastGrid.cols, lastGrid.rows, feed.channel()),
          );
          if (disposed) return;
          id = opened;
          idRef.current = id;
          onStartedRef.current?.(opened);
        } catch (e) {
          log.error("script-run", `starting the ${kind} script failed`, e);
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
        streaming = true;
        lastGrid = engine.fit();
        void commands.terminalResize(id, lastGrid.cols, lastGrid.rows);
      }
    })();

    const unsubInput = engine.onInput((data) => {
      if (id) writeTerminal(id, data);
    });

    // The stream died with the old connection; a run that ended meanwhile shows through the refetched scripts.
    const offReconnect = onServerReconnected(() => {
      if (disposed || !streaming || ended || !id) return;
      const { from, channel } = feed.resume();
      void unwrap(commands.terminalAttach(id, from, channel)).catch((e) =>
        log.warn("script-run", "reattaching after a reconnect failed", e),
      );
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
      offReconnect();
      ro.disconnect();
      engineRef.current = null;
      idRef.current = undefined;
      engine.dispose();
      feed.release();
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
      {sizeOwner === false && <TakeSizeButton terminalId={() => idRef.current} />}
    </div>
  );
}
