import { Terminal, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import "@xterm/xterm/css/xterm.css";

import { log } from "@/lib/log";

export type TerminalTheme = ITheme;

export type TerminalGrid = { cols: number; rows: number };

export interface TerminalEngine {
  mount(host: HTMLElement): void;
  write(data: Uint8Array | string): void;
  onInput(cb: (data: string) => void): () => void;
  onKey(cb: (e: { key: string; domEvent: KeyboardEvent }) => void): () => void;
  fit(): TerminalGrid;
  refresh(): void;
  grid(): TerminalGrid;
  setFontSize(px: number): TerminalGrid;
  setTheme(theme: TerminalTheme): void;
  focus(): void;
  hasSelection(): boolean;
  selectionText(): string;
  paste(text: string): void;
  interceptKeys(filter: (e: KeyboardEvent) => boolean): void;
  reset(): void;
  dispose(): void;
}

const FIT_RETRY_LIMIT = 20;

const EMPTY_GRID: TerminalGrid = { cols: 0, rows: 0 };

type XtermCore = {
  _core: {
    _createRenderer: () => unknown;
    _renderService: {
      setRenderer: (renderer: unknown) => void;
      handleResize: (cols: number, rows: number) => void;
    };
  };
};

// @xterm/addon-webgl 0.19 releases its GPU context only when the detached canvas
// is collected, and WebKit silently marks the oldest context lost once a web
// process holds ~16 of them — blanking whichever terminal owned it.
function releaseWebglContext(canvas: HTMLCanvasElement | null): void {
  if (!canvas) return;
  try {
    canvas.getContext("webgl2")?.getExtension("WEBGL_lose_context")?.loseContext();
  } catch {
  }
}

class XtermEngine implements TerminalEngine {
  private term: Terminal | null;
  private fitAddon: FitAddon | null;
  private webgl?: WebglAddon;
  private rendererReady = false;
  private disposed = false;
  private fitRetries = 0;

  constructor(opts: { fontSize: number; theme: TerminalTheme; cursorBlink?: boolean }) {
    this.term = new Terminal({
      fontSize: opts.fontSize,
      fontFamily: '"JetBrains Mono", ui-monospace, monospace',
      fontWeight: 400,
      // Only 400/500/600 are bundled; 700 would be synthesized.
      fontWeightBold: 600,
      lineHeight: 1.0,
      theme: opts.theme,
      cursorBlink: opts.cursorBlink ?? true,
      scrollback: 5000,
      convertEol: false,
      allowProposedApi: true,
    });
    this.fitAddon = new FitAddon();
    this.term.loadAddon(this.fitAddon);
  }

  mount(host: HTMLElement): void {
    if (!this.term) return;
    this.term.open(host);
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => {
        log.info("terminal", "WebGL context lost; switching to the DOM renderer");
        const canvas = this.webglCanvas();
        try {
          webgl.dispose();
        } catch {
        }
        if (this.webgl === webgl) this.webgl = undefined;
        releaseWebglContext(canvas);
        this.restoreDomRenderer();
      });
      this.term.loadAddon(webgl);
      this.webgl = webgl;
    } catch (e) {
      log.info("terminal", "WebGL unavailable; using the DOM renderer", e);
    }
  }

  // The addon's own DOM-renderer restore is gated on `_core._store._isDisposed`,
  // a field @xterm/xterm 5.5 does not have, so it throws *after* tearing the WebGL
  // canvas down and leaves the terminal with no renderer at all — blank for good.
  // Verified against @xterm/xterm 5.5.0 + @xterm/addon-webgl 0.19.0.
  private restoreDomRenderer(): void {
    if (this.disposed || !this.term) return;
    try {
      const core = (this.term as unknown as XtermCore)._core;
      core._renderService.setRenderer(core._createRenderer());
      core._renderService.handleResize(this.term.cols, this.term.rows);
      this.rendererReady = false;
      this.fitRetries = 0;
      this.fit();
    } catch (e) {
      log.error("terminal", "restoring the DOM renderer failed; the terminal stays blank", e);
    }
  }

  private webglCanvas(): HTMLCanvasElement | null {
    const screen = this.term?.element?.querySelector(".xterm-screen");
    if (!screen) return null;
    return [...screen.querySelectorAll("canvas")].find((c) => !c.className) ?? null;
  }

  write(data: Uint8Array | string): void {
    this.term?.write(data);
  }

  onInput(cb: (data: string) => void): () => void {
    const sub = this.term?.onData(cb);
    return () => sub?.dispose();
  }

  onKey(cb: (e: { key: string; domEvent: KeyboardEvent }) => void): () => void {
    const sub = this.term?.onKey(cb);
    return () => sub?.dispose();
  }

  fit(): TerminalGrid {
    // fitAddon.fit() reads xterm's renderer dimensions, which aren't set
    // until the renderer has completed its first paint — a race a fast
    // programmatic mount can lose in a way a human's slower click-then-look
    // cadence usually doesn't. The previous code set rendererReady only
    // inside this same try, so a single miss disabled refresh() forever.
    // Retry on the next frame instead, bounded, and self-heal once ready.
    if (!this.fitAddon) return EMPTY_GRID;
    try {
      this.fitAddon.fit();
      this.rendererReady = true;
      this.fitRetries = 0;
    } catch {
      if (!this.rendererReady && this.fitRetries < FIT_RETRY_LIMIT) {
        this.fitRetries += 1;
        requestAnimationFrame(() => {
          if (this.disposed) return;
          this.fit();
          if (this.rendererReady) this.refresh();
        });
      }
    }
    return this.grid();
  }

  grid(): TerminalGrid {
    if (!this.term) return EMPTY_GRID;
    return { cols: this.term.cols, rows: this.term.rows };
  }

  refresh(): void {
    if (!this.rendererReady || !this.term) return;
    this.term.refresh(0, this.term.rows - 1);
  }

  setFontSize(px: number): TerminalGrid {
    if (!this.term) return EMPTY_GRID;
    this.term.options.fontSize = px;
    return this.fit();
  }

  setTheme(theme: TerminalTheme): void {
    if (!this.term) return;
    this.term.options.theme = theme;
  }

  focus(): void {
    this.term?.focus();
  }

  hasSelection(): boolean {
    return this.term?.hasSelection() ?? false;
  }

  selectionText(): string {
    return this.term?.getSelection() ?? "";
  }

  paste(text: string): void {
    this.term?.paste(text);
  }

  interceptKeys(filter: (e: KeyboardEvent) => boolean): void {
    this.term?.attachCustomKeyEventHandler(filter);
  }

  reset(): void {
    this.term?.reset();
  }

  // Dropping both references is what actually frees the scrollback buffer: an
  // attach callback that outlives this engine keeps the engine reachable, and
  // `FitAddon.dispose()` is an empty method that leaves the addon's own
  // `_terminal` field pointing at the Terminal. Verified against
  // @xterm/addon-fit 0.11.0.
  dispose(): void {
    this.disposed = true;
    const canvas = this.webglCanvas();
    // xterm 5.5 may throw while reverting the WebGL renderer.
    try {
      this.webgl?.dispose();
    } catch {
    }
    this.webgl = undefined;
    releaseWebglContext(canvas);
    this.term?.dispose();
    this.term = null;
    this.fitAddon = null;
  }
}

export function createTerminalEngine(opts: {
  fontSize: number;
  theme: TerminalTheme;
  cursorBlink?: boolean;
}): TerminalEngine {
  return new XtermEngine(opts);
}
