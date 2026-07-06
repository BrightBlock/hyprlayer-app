import { useEffect, useRef } from "react";
import { Channel } from "@tauri-apps/api/core";

import { commands, type ScriptKind, type TerminalEvent } from "@/lib/bindings";
import { unwrap } from "@/lib/ipc";
import { b64ToBytes, strToB64 } from "@/lib/terminal/terminalCodec";
import { createTerminalEngine } from "@/lib/terminal/terminalEngine";
import { useTerminalFontSize } from "@/lib/terminal/terminalFontSize";
import { xtermThemeFromCss } from "@/lib/terminal/terminalTheme";

type Props = {
  workstreamId: string;
  kind: ScriptKind;
  /** A live PTY to attach to (replays scrollback + tails); null → start fresh. */
  attachId: string | null;
  /** Bump to force a fresh run (▶ / restart). The effect keys on this. */
  gen: number;
  /** Fired with the new terminal id once a fresh run spawns. */
  onStarted?: (terminalId: string) => void;
  /** Fired when the process exits (finished or stopped). */
  onExit?: () => void;
};

/**
 * Streams one script run into an xterm view — either attaching to a live PTY or
 * spawning a fresh one via `script_run`. Reuses the same engine as
 * {@link TerminalTab}; trimmed of zoom/live-theme/focus-repaint (the run log
 * self-heals on the next output). Unmounting never kills the PTY — collapsing the
 * dock keeps a Run alive (the rail's green dot is the signal); teardown is the
 * backend's job (Stop, archive, or app exit).
 */
export function ScriptRunView({ workstreamId, kind, attachId, gen, onStarted, onExit }: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const fontSize = useTerminalFontSize((s) => s.fontSize);
  const attachRef = useRef(attachId);
  attachRef.current = attachId;
  const onStartedRef = useRef(onStarted);
  onStartedRef.current = onStarted;
  const onExitRef = useRef(onExit);
  onExitRef.current = onExit;

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const engine = createTerminalEngine({ fontSize, theme: xtermThemeFromCss() });
    engine.mount(host);
    let lastGrid = engine.fit();
    engine.focus();

    let disposed = false;
    let id: string | undefined = attachRef.current ?? undefined;

    const newChannel = (): Channel<TerminalEvent> => {
      const channel = new Channel<TerminalEvent>();
      channel.onmessage = (ev) => {
        if (ev.kind === "output") engine.write(b64ToBytes(ev.data));
        else if (ev.kind === "exited") {
          engine.write("\r\n\x1b[2m[process exited]\x1b[0m\r\n");
          onExitRef.current?.();
        }
      };
      return channel;
    };

    void (async () => {
      if (id) {
        const ok = await unwrap(commands.terminalAttach(id, newChannel()));
        if (!ok) id = undefined; // PTY gone → start fresh below
      }
      if (!id && !disposed) {
        // Wait two frames so xterm has measured its real grid before the PTY
        // opens at that size (see TerminalTab for the "garbage dims" rationale).
        await new Promise<void>((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
        );
        if (disposed) return;
        lastGrid = engine.fit();
        try {
          const opened = await unwrap(
            commands.scriptRun(workstreamId, kind, lastGrid.cols, lastGrid.rows, newChannel()),
          );
          // If we unmounted mid-spawn, leave the run alive (collapse keeps it
          // running); the backend tracks it and the rail shows it's live.
          if (disposed) return;
          id = opened;
          onStartedRef.current?.(opened);
        } catch (e) {
          engine.write(`\r\n\x1b[31m${e instanceof Error ? e.message : String(e)}\x1b[0m\r\n`);
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
      engine.dispose();
    };
    // Re-mount on a real identity change or an explicit restart (`gen`). attachId
    // is read once at mount via the ref so a post-start id change can't remount.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workstreamId, gen]);

  return (
    <div className="relative min-h-0 min-w-0 flex-1 bg-surface">
      <div ref={hostRef} className="absolute inset-2 overflow-hidden" />
    </div>
  );
}
