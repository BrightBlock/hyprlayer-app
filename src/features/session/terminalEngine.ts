import { Terminal, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
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

  constructor(opts: { fontSize: number; theme: TerminalTheme }) {
    this.term = new Terminal({
      fontSize: opts.fontSize,
      fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
      theme: opts.theme,
      cursorBlink: true,
      scrollback: 5000,
      convertEol: false,
    });
    this.fitAddon = new FitAddon();
    this.term.loadAddon(this.fitAddon);
  }

  mount(host: HTMLElement): void {
    this.term.open(host);
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
    } catch {
      // renderer not ready yet — keep the current (default) grid for now
    }
    return this.grid();
  }

  grid(): TerminalGrid {
    return { cols: this.term.cols, rows: this.term.rows };
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
