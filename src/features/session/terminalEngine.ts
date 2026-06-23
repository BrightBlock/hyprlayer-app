import { Terminal, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import "@xterm/xterm/css/xterm.css";

/**
 * The seam that lets the terminal UI depend on an abstract emulator instead of
 * `@xterm/xterm` directly. The xterm-backed implementation lives here; a future
 * libghostty `ghostty-web` engine — its `Terminal`/`write`/`onData` API is
 * deliberately xterm-compatible — can implement the same interface and be
 * swapped in via `createTerminalEngine` with no `TerminalTab` churn.
 */

/** The renderer-agnostic theme the app feeds an engine — xterm's `ITheme`
 *  today; a GhosttyWebEngine would map this onto its own palette. */
export type TerminalTheme = ITheme;

export type TerminalGrid = { cols: number; rows: number };

export interface TerminalEngine {
  /** Mount into a host element. Call once, before any other method. */
  mount(host: HTMLElement): void;
  /** Render emulator output: raw PTY bytes, or a UTF-8 control string. */
  write(data: Uint8Array | string): void;
  /** Subscribe to user input (keystrokes as a string); returns an unsubscribe. */
  onInput(cb: (data: string) => void): () => void;
  /** Refit to the host's current size; returns the resulting grid. */
  fit(): TerminalGrid;
  /** Force a full repaint of the visible rows, bypassing the staleness that a
   *  throttled-while-unfocused rAF can leave behind. */
  refresh(): void;
  /** The current grid, without refitting. */
  grid(): TerminalGrid;
  /** Change the font size (px) and refit; returns the resulting grid. */
  setFontSize(px: number): TerminalGrid;
  /** Recolor the emulator. */
  setTheme(theme: TerminalTheme): void;
  /** Give the emulator keyboard focus. */
  focus(): void;
  /** Tear down and release resources. */
  dispose(): void;
}

class XtermEngine implements TerminalEngine {
  private readonly term: Terminal;
  private readonly fitAddon: FitAddon;
  /** The GPU renderer, while it's live. Disposed on context loss (xterm then
   *  falls back to its DOM renderer) and on teardown. */
  private webgl?: WebglAddon;
  /** Pending atlas-rebuild timers, cleared on dispose. */
  private rebuildTimers: number[] = [];
  /** Flips true the first time `fit()` reads the renderer without throwing —
   *  proof the DOM renderer is live. Gates `refresh()`, whose `term.refresh()`
   *  schedules an rAF render that reads renderer dimensions and throws
   *  (uncaught, in the frame callback) if the renderer isn't set up yet. */
  private rendererReady = false;

  constructor(opts: { fontSize: number; theme: TerminalTheme }) {
    this.term = new Terminal({
      fontSize: opts.fontSize,
      fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
      theme: opts.theme,
      cursorBlink: true,
      scrollback: 5000,
      convertEol: false,
      // The WebGL renderer needs the proposed-API surface (xterm gates the
      // renderer-internals it reaches for behind this flag).
      allowProposedApi: true,
    });
    this.fitAddon = new FitAddon();
    this.term.loadAddon(this.fitAddon);
  }

  mount(host: HTMLElement): void {
    this.term.open(host);
    // Drive the grid through the WebGL renderer: it paints to a single
    // GPU-composited <canvas> rather than per-row DOM nodes. WKWebView composites
    // a canvas reliably, whereas the DOM renderer's incremental row mutations can
    // be left un-repainted after a wrap/scroll — the terminal goes black until a
    // resize forces a full re-composite. Must load AFTER open() (it needs the
    // screen element + render service). If the GPU context drops (WKWebView can
    // shed it when backgrounded), dispose the addon so xterm reverts to the DOM
    // renderer instead of freezing on a dead canvas. Guarded: if WebGL can't
    // initialise at all, the DOM renderer remains and the terminal still works.
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => {
        webgl.dispose();
        if (this.webgl === webgl) this.webgl = undefined;
      });
      this.term.loadAddon(webgl);
      this.webgl = webgl;
    } catch {
      // WebGL unavailable — keep the DOM renderer.
    }
    this.scheduleAtlasRebuild();
  }

  /**
   * Rebuild the WebGL glyph atlas once layout + DPR have settled. The atlas is
   * rasterised from the renderer's metrics the first time it paints; on a fresh
   * pane those can still be settling, and because BOTH xterm's `FitAddon` and our
   * `ResizeObserver` no-op when the grid is unchanged, a stale atlas would never
   * rebuild on its own — glyphs render blank/misaligned and the line runs off the
   * edge without wrapping until a manual resize forces a rebuild (the reported
   * bug). Mirroring termic, re-settle across the next few frames + a couple of
   * timeouts: `clearTextureAtlas()` drops the stale atlas and `refresh()` repaints
   * the whole viewport, so it comes back correct with no user resize. DOM-renderer
   * sessions (WebGL unavailable) have no atlas, so this is a WebGL-only nudge.
   */
  private scheduleAtlasRebuild(): void {
    const rebuild = () => {
      if (!this.webgl) return;
      try {
        this.term.clearTextureAtlas();
        this.term.refresh(0, this.term.rows - 1);
      } catch {
        // renderer mid-teardown — nothing to rebuild
      }
    };
    requestAnimationFrame(() => requestAnimationFrame(rebuild));
    this.rebuildTimers = [
      window.setTimeout(rebuild, 250),
      window.setTimeout(rebuild, 600),
    ];
  }

  write(data: Uint8Array | string): void {
    this.term.write(data);
  }

  onInput(cb: (data: string) => void): () => void {
    const sub = this.term.onData(cb);
    return () => sub.dispose();
  }

  fit(): TerminalGrid {
    // FitAddon.fit() reaches into the render dimensions, which xterm only
    // computes on its first paint — deferred a frame after open(). A fit() in
    // the same tick as mount therefore throws ("_renderer.value.dimensions").
    // Swallow it and return the current grid; the ResizeObserver's initial
    // callback re-fits once the renderer is live, correcting the PTY size. Left
    // unguarded, that throw would abort TerminalTab's setup effect mid-way and
    // the PTY would never open (blank terminal, just a blinking cursor).
    try {
      this.fitAddon.fit();
      // `fitAddon.fit()` reads `_renderService.dimensions` first thing; reaching
      // here without throwing means the renderer is live.
      this.rendererReady = true;
    } catch {
      // renderer not ready yet — keep the current (default) grid for now
    }
    return this.grid();
  }

  grid(): TerminalGrid {
    return { cols: this.term.cols, rows: this.term.rows };
  }

  refresh(): void {
    // xterm's DOM renderer flushes paints on requestAnimationFrame, which a
    // WKWebView throttles (and can drop) while the window is unfocused — so a
    // scroll that lands while unfocused leaves the screen stale, and it doesn't
    // self-heal on refocus (the dropped frame is never rescheduled). Marking all
    // rows dirty here re-queues a paint, which fires immediately once focus (and
    // a live rAF) is back — the programmatic equivalent of nudging a resize.
    //
    // Gate on a live renderer: `term.refresh()` schedules an rAF render that
    // reads the renderer's dimensions, which throws *uncaught* (inside the frame
    // callback, past any try/catch here) if the renderer isn't set up yet. A
    // focus event that lands before the first fit would otherwise do exactly
    // that. A successful fit() is our proof the renderer exists.
    if (!this.rendererReady) return;
    this.term.refresh(0, this.term.rows - 1);
  }

  setFontSize(px: number): TerminalGrid {
    this.term.options.fontSize = px;
    return this.fit();
  }

  setTheme(theme: TerminalTheme): void {
    this.term.options.theme = theme;
  }

  focus(): void {
    this.term.focus();
  }

  dispose(): void {
    for (const t of this.rebuildTimers) window.clearTimeout(t);
    this.rebuildTimers = [];
    // Dispose the GPU renderer before the terminal so its canvas/observers are
    // released cleanly.
    this.webgl?.dispose();
    this.webgl = undefined;
    this.term.dispose();
  }
}

/** Build the active terminal engine. Swapping the implementation here migrates
 *  the whole app to a different emulator (e.g. ghostty-web) in one place. */
export function createTerminalEngine(opts: {
  fontSize: number;
  theme: TerminalTheme;
}): TerminalEngine {
  return new XtermEngine(opts);
}
