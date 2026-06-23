import { useEffect, useRef } from "react";
import { Channel } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { commands, type TerminalEvent } from "../../lib/bindings";
import { unwrap } from "../../lib/ipc";
import { useCommand } from "../../lib/keybindings/registry";
import { useThemeStore } from "../../lib/theme";
import { createTerminalEngine, type TerminalEngine } from "./terminalEngine";
import { useTerminalFontSize } from "./terminalFontSize";
import { xtermThemeFromCss } from "./terminalTheme";

const b64ToBytes = (s: string): Uint8Array =>
  Uint8Array.from(atob(s), (c) => c.charCodeAt(0));

const strToB64 = (s: string): string => {
  const bytes = new TextEncoder().encode(s);
  // Build the binary string in chunks: `String.fromCharCode(...bytes)` spreads
  // every byte as an argument, which overflows the call stack on a large paste.
  let bin = "";
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    bin += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(bin);
};

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
  // Refs let the font-size / theme effects reach the live engine without
  // re-running (and tearing down) the main setup effect.
  const engineRef = useRef<TerminalEngine | null>(null);
  const idRef = useRef<string | undefined>(terminalId);
  const appliedFontSizeRef = useRef<number>(0);

  const fontSize = useTerminalFontSize((s) => s.fontSize);
  const zoomIn = useTerminalFontSize((s) => s.zoomIn);
  const zoomOut = useTerminalFontSize((s) => s.zoomOut);
  const zoomReset = useTerminalFontSize((s) => s.reset);

  // Zoom acts on whichever terminal is mounted — registering here is the
  // context gate, so ⌘+/⌘-/⌘0 are inert when no terminal is on screen.
  useCommand("terminal.zoomIn", zoomIn);
  useCommand("terminal.zoomOut", zoomOut);
  useCommand("terminal.zoomReset", zoomReset);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const engine = createTerminalEngine({ fontSize, theme: xtermThemeFromCss() });
    engine.mount(host);
    engineRef.current = engine;
    appliedFontSizeRef.current = fontSize;
    // Effects run after layout, so the host has its real size here; fit once
    // synchronously so the PTY opens at the correct grid below.
    let lastGrid = engine.fit();

    // Reveal paths (⌘` toggle, tab switch, first open) remount this component
    // with its pane active, so grabbing focus on mount lets the user type
    // immediately without clicking in. Gated on `autoFocus` (read once at mount)
    // so showing a dock whose sibling pane is active doesn't steal focus.
    if (autoFocus) engine.focus();

    // A Tauri Channel can back exactly ONE invoke: when that command's backend
    // `Channel` drops, it emits an `end` marker, and the frontend `Channel`
    // responds by unregistering its own callback (see `cleanupCallback` in
    // @tauri-apps/api `core`). A failed `terminal_attach` drops its channel the
    // instant it returns `false` — so reusing that same channel for the
    // follow-up `terminal_open` sends the fresh shell's prompt into an already
    // torn-down callback: it's dropped, and the terminal sits blank with only a
    // blinking cursor (the resume-after-restart path, where the persisted id is
    // dead). Mint a fresh channel per invoke; all route to the same engine.
    const newChannel = (): Channel<TerminalEvent> => {
      const channel = new Channel<TerminalEvent>();
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
          id = undefined; // PTY gone (e.g. restart) → open fresh
          idRef.current = undefined;
        }
      }
      if (!id && !disposed) {
        // Spawn the PTY at the TRUE grid, not xterm's 80×24 default. xterm only
        // computes its render dimensions a frame after open(), so the mount-time
        // fit() no-ops and `engine.grid()` is still 80×24 here. Opening the PTY at
        // that wrong size makes the shell's line editor (zle) initialise its width
        // at 80; the later resize to the real width doesn't cleanly reflow it, so
        // a command long enough to wrap corrupts the prompt redraw and the line
        // blanks mid-screen — only fixed by re-execing the shell (`exec zsh -l`).
        // Wait two frames for the renderer, then fit, so the shell starts at the
        // real width. (termic defers its spawn for exactly this "garbage dims"
        // reason.)
        await new Promise<void>((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
        );
        if (disposed) return;
        lastGrid = engine.fit();
        const opened = await unwrap(
          commands.terminalOpen(workstreamId, lastGrid.cols, lastGrid.rows, newChannel()),
        );
        // The tab may have unmounted while `terminalOpen` was in flight. The
        // cleanup can't kill a PTY it never saw an id for, so close it here and
        // skip publishing the id upstream.
        if (disposed) {
          void commands.terminalClose(opened);
          return;
        }
        id = opened;
        idRef.current = id;
        onTerminalId(id);
      }
      // Reconcile once more now that `id` is live, covering the attach path and
      // any layout that settled during the await above. A no-op on the backend
      // when the size already matches.
      if (id && !disposed) {
        lastGrid = engine.fit();
        void commands.terminalResize(id, lastGrid.cols, lastGrid.rows);
      }
    })();

    const unsubInput = engine.onInput((data) => {
      if (id) void commands.terminalWrite(id, strToB64(data));
    });

    // Refit on container resize, but coalesce to a frame and only emit a PTY
    // resize when the grid actually changed. The host is absolutely positioned
    // (see render) so fitting the terminal cannot grow the layout box — without
    // that decoupling, fit → DOM resize → ResizeObserver → fit oscillates and
    // visibly shakes the surrounding panes.
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

    // xterm paints on requestAnimationFrame, which a WKWebView throttles (and
    // can drop) while the window is unfocused — so output or a scroll that lands
    // while you're away can leave the terminal stale, and it doesn't self-heal on
    // refocus (the user's workaround was to resize the pane, which forces a
    // repaint via the grid change). On focus-regain, refit (in case the layout
    // drifted) and force a repaint so the terminal is correct the instant you
    // look back, with no resize needed.
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
    };
    // Re-mount only on a real identity change (workstream / shell). `terminalId`
    // is read once at mount (the closure captures it) and deliberately NOT a
    // dependency: this effect *publishes* the id via onTerminalId after opening,
    // so depending on it would tear down and remount the engine the instant the
    // first open completes. onTerminalId is stable; fontSize / theme are applied
    // by the effects below rather than by re-creating the engine. `autoFocus` is
    // likewise read once at mount (a later focus change must not re-grab focus).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workstreamId, shellId]);

  // Apply zoom in place: change the font, re-fit to the new cell size, and tell
  // the PTY its new grid. Skips the no-op initial run (engine built at fontSize).
  useEffect(() => {
    const engine = engineRef.current;
    if (!engine || appliedFontSizeRef.current === fontSize) return;
    appliedFontSizeRef.current = fontSize;
    const g = engine.setFontSize(fontSize);
    if (idRef.current) void commands.terminalResize(idRef.current, g.cols, g.rows);
  }, [fontSize]);

  // Recolor live when the theme changes. Re-read on the next frame so the
  // <html> theme class (toggled by the theme store's own subscriber) has applied
  // before we sample the CSS variables; also covers "system" → OS-appearance.
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

  // Outer box flows in the flex column; the inner host is absolutely positioned
  // so the terminal's rendered size never feeds back into the page layout. The
  // 8px breathing room is positional `inset-2`, NOT padding: FitAddon measures
  // the host via getComputedStyle().height, which reports the border-box height
  // here — so padding would inflate its row count and clip the last line below
  // the viewport. With no padding, the measured height matches the content box.
  return (
    <div className="relative min-h-0 min-w-0 flex-1 bg-surface">
      <div ref={hostRef} className="absolute inset-2 overflow-hidden" />
    </div>
  );
}
