import { Terminal, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import "@xterm/xterm/css/xterm.css";

/**
 * The seam that lets the terminal UI depend on an abstract emulator instead of
 * `@xterm/xterm` directly. The xterm-backed implementation lives here. Another
 * engine exposing an xterm-compatible `Terminal`/`write`/`onData` API can
 * implement the same interface and be swapped in via `createTerminalEngine`
 * with no `TerminalTab` churn.
 */

/** The renderer-agnostic theme the app feeds an engine, xterm's `ITheme`
 *  today. Another engine would map this onto its own palette. */
export type TerminalTheme = ITheme;

export type TerminalGrid = { cols: number; rows: number };

export interface TerminalEngine {
  /** Mount into a host element. Call once, before any other method. */
  mount(host: HTMLElement): void;
  /** Render emulator output: raw PTY bytes, or a UTF-8 control string. */
  write(data: Uint8Array | string): void;
  /** Subscribe to all input bound for the PTY, keystrokes, paste, AND xterm's
   *  automatic replies to the CLI's capability queries (DSR/DA). Returns an
   *  unsubscribe. Write every byte to the PTY; don't treat it as "the user did
   *  something" (those auto-replies fire with no user action, use `onKey`). */
  onInput(cb: (data: string) => void): () => void;
  /** Subscribe to real key presses only (not paste or terminal auto-replies),
   *  the honest "the user is typing" signal. Returns an unsubscribe. */
  onKey(cb: (e: { key: string; domEvent: KeyboardEvent }) => void): () => void;
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
  /** True when the emulator holds a text selection. This is the emulator's own
   *  selection model, not a DOM one, the grid is painted to a canvas, so
   *  `window.getSelection()` never sees it. */
  hasSelection(): boolean;
  /** The selected text, or `""` when nothing is selected. */
  selectionText(): string;
  /** Feed text to the PTY exactly as a real paste would: line endings
   *  normalized, and bracketed-paste wrapped when the running CLI asked for it. */
  paste(text: string): void;
  /** Install a keydown/keypress filter that runs BEFORE the emulator's own
   *  handling. Return true to let the emulator proceed, false to claim the key
   *  (the emulator then does nothing and does not preventDefault, the filter
   *  owns that). One filter per engine; call after `mount`. */
  interceptKeys(filter: (e: KeyboardEvent) => boolean): void;
  /** Wipe the screen + scrollback to a clean slate (used when restarting an
   *  agent so the prior session's output doesn't linger). */
  reset(): void;
  /** Tear down and release resources. */
  dispose(): void;
}

class XtermEngine implements TerminalEngine {
  private readonly term: Terminal;
  private readonly fitAddon: FitAddon;
  /** The GPU renderer, while it's live. Disposed on context loss (xterm then
   *  falls back to its DOM renderer) and on teardown. */
  private webgl?: WebglAddon;
  /** Flips true the first time `fit()` reads the renderer without throwing,
   *  proof the renderer is live. Gates `refresh()`, whose `term.refresh()`
   *  schedules an rAF render that reads renderer dimensions and throws
   *  (uncaught, in the frame callback) if the renderer isn't set up yet. */
  private rendererReady = false;

  constructor(opts: { fontSize: number; theme: TerminalTheme; cursorBlink?: boolean }) {
    this.term = new Terminal({
      fontSize: opts.fontSize,
      // Bundled first: `ui-monospace` resolves to a *proportional* face through
      // fontconfig on a typical Linux box (see the typography block in index.css).
      fontFamily:
        '"JetBrains Mono Variable", ui-monospace, SFMono-Regular, Menlo, monospace',
      // Pin the weights to 400 / 700: any other weight makes xterm's WebGL atlas
      // interpolate, which WKWebView rasterizes soft/garbled. lineHeight 1.0 is
      // what TUIs (claude/codex/gemini) assume, a larger value inflates each
      // cell and shows "ribbon" gaps between bg-colored rows.
      fontWeight: 400,
      fontWeightBold: 700,
      lineHeight: 1.0,
      theme: opts.theme,
      // Default on; a caller steadies it per-engine. Codex parks a *visible*
      // cursor in its footer while mid-turn (other CLIs hide theirs), so blinking
      // it there reads as a flicker.
      cursorBlink: opts.cursorBlink ?? true,
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
    // GPU-composited <canvas> rather than per-row DOM nodes, which WKWebView
    // composites reliably (the DOM renderer can leave wraps/scrolls un-repainted
    // until a resize). Load AFTER open() so the GL context attaches to an
    // already-laid-out canvas (a 0×0 canvas rasterizes the glyph atlas wrong →
    // scrambled text). If the GPU context drops (WKWebView sheds it when
    // backgrounded), dispose the addon so xterm reverts to the DOM renderer
    // instead of freezing on a dead canvas. Guarded: if WebGL can't initialise,
    // the DOM renderer remains and the terminal still works. (addon-webgl ≥0.19
    // manages its own glyph atlas across resizes, no manual clearTextureAtlas.)
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => {
        // Same guard as dispose(): the addon's revert-on-dispose throws on xterm 5.5.
        try {
          webgl.dispose();
        } catch {
          // addon revert-on-dispose hit a missing xterm-5.5 core internal
        }
        if (this.webgl === webgl) this.webgl = undefined;
      });
      this.term.loadAddon(webgl);
      this.webgl = webgl;
    } catch {
      // WebGL unavailable, keep the DOM renderer.
    }
  }

  write(data: Uint8Array | string): void {
    this.term.write(data);
  }

  onInput(cb: (data: string) => void): () => void {
    const sub = this.term.onData(cb);
    return () => sub.dispose();
  }

  onKey(cb: (e: { key: string; domEvent: KeyboardEvent }) => void): () => void {
    const sub = this.term.onKey(cb);
    return () => sub.dispose();
  }

  fit(): TerminalGrid {
    // FitAddon.fit() reaches into the render dimensions, which xterm only
    // computes on its first paint, deferred a frame after open(). A fit() in
    // the same tick as mount therefore throws ("_renderer.value.dimensions").
    // Swallow it and return the current grid; the ResizeObserver's initial
    // callback re-fits once the renderer is live, correcting the PTY size. Left
    // unguarded, that throw would abort the setup effect mid-way and the PTY
    // would never open (blank terminal, just a blinking cursor).
    try {
      this.fitAddon.fit();
      // Reaching here without throwing means the renderer is live.
      this.rendererReady = true;
    } catch {
      // renderer not ready yet, keep the current (default) grid for now
    }
    return this.grid();
  }

  grid(): TerminalGrid {
    return { cols: this.term.cols, rows: this.term.rows };
  }

  refresh(): void {
    // xterm flushes paints on requestAnimationFrame, which a WKWebView throttles
    // (and can drop) while the window is unfocused, so output or a scroll that
    // lands while you're away leaves the screen stale and doesn't self-heal on
    // refocus. Marking all rows dirty re-queues a paint, which fires the instant
    // focus (and a live rAF) is back. Gated on a live renderer: term.refresh()
    // schedules an rAF render that reads renderer dimensions and throws
    // *uncaught* (in the frame callback, past any try/catch here) if the
    // renderer isn't set up yet. A successful fit() is our proof it exists.
    if (!this.rendererReady) return;
    this.term.refresh(0, this.term.rows - 1);
  }

  setFontSize(px: number): TerminalGrid {
    // Changing the option re-rasterises the renderer's glyph atlas at the new
    // cell size on its own; just refit so the grid (and PTY) follow.
    this.term.options.fontSize = px;
    return this.fit();
  }

  setTheme(theme: TerminalTheme): void {
    this.term.options.theme = theme;
  }

  focus(): void {
    this.term.focus();
  }

  hasSelection(): boolean {
    return this.term.hasSelection();
  }

  selectionText(): string {
    return this.term.getSelection();
  }

  paste(text: string): void {
    this.term.paste(text);
  }

  interceptKeys(filter: (e: KeyboardEvent) => boolean): void {
    this.term.attachCustomKeyEventHandler(filter);
  }

  reset(): void {
    this.term.reset();
  }

  dispose(): void {
    // Dispose the GPU renderer before the terminal so its canvas/observers are
    // released cleanly. Guard it: addon-webgl's revert-on-dispose callback reads
    // `_core._store._isDisposed`, which xterm 5.5's core doesn't expose → it
    // throws. The terminal is torn down next, so the revert is moot.
    try {
      this.webgl?.dispose();
    } catch {
      // addon revert-on-dispose hit a missing xterm-5.5 core internal
    }
    this.webgl = undefined;
    this.term.dispose();
  }
}

/** Build the active terminal engine. Swapping the implementation here migrates
 *  the whole app to a different emulator in one place. */
export function createTerminalEngine(opts: {
  fontSize: number;
  theme: TerminalTheme;
  cursorBlink?: boolean;
}): TerminalEngine {
  return new XtermEngine(opts);
}
