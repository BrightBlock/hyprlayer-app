import { Terminal, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { WebglAddon } from "@xterm/addon-webgl";
import "@xterm/xterm/css/xterm.css";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";

import { openExternalUrl } from "@/lib/browser";
import { log } from "@/lib/log";
import { b64ToBytes } from "./terminalCodec";
import { isDarkBackground } from "./terminalTheme";

export type TerminalTheme = ITheme;

export type TerminalGrid = { cols: number; rows: number };

export interface TerminalEngine {
  mount(host: HTMLElement): void;
  /** Ties the engine to a pty, so what its capped replay lost carries over from earlier engines. */
  bind(terminalId: string): void;
  write(data: Uint8Array | string): void;
  writeFrame(data: Uint8Array, written: () => void): void;
  clipboard(data: string): void;
  /** Renders at the pty's size from here in the output on, and stops fitting its host. */
  follow(grid: TerminalGrid): void;
  onInput(cb: (data: string) => void): () => void;
  onKey(cb: (e: { key: string; domEvent: KeyboardEvent }) => void): () => void;
  /** The grid that fits the host; the terminal takes it only until it follows a pty's size. */
  fit(): TerminalGrid;
  refresh(): void;
  grid(): TerminalGrid;
  setFontSize(px: number): TerminalGrid;
  setFontFamily(family: string): Promise<TerminalGrid>;
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

const THEME_REPORTS_MODE = 2031;
const COLOR_SCHEME_QUERY = 996;
const CLIPBOARD_OSC = 52;
// The browsers' transient-activation window: room for a copy-on-select or `/copy` to come back over ssh.
const USER_GESTURE_MS = 5000;

// Ptys whose program asked for theme reports; its `CSI ? 2031 h` can scroll out of a replay.
const themeReporting = new Set<string>();

// xterm's defaults go through confirm() and window.open(), and neither reaches the browser here.
function openLink(_event: MouseEvent, uri: string): void {
  void openExternalUrl(uri).catch((e) => log.warn("terminal", "opening a terminal link failed", e));
}

function copyFromProgram(base64: string): void {
  try {
    const text = new TextDecoder().decode(b64ToBytes(base64));
    void writeText(text).catch((e) => log.warn("terminal", "copying for a program failed", e));
  } catch {
  }
}

type XtermCore = {
  _core: {
    viewport?: { syncScrollArea: () => void };
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
  private fontFamilyRequest = "";
  private dark: boolean;
  private reportsTheme = false;
  private terminalId?: string;
  // Set by bind: the replay that follows is not yet written, then not yet parsed.
  private replay: "awaiting" | "parsing" | null = null;
  private lastGesture = -Infinity;
  private following = false;
  private fitted: TerminalGrid | null = null;
  private frames = false;

  constructor(opts: {
    fontSize: number;
    fontFamily: string;
    theme: TerminalTheme;
    cursorBlink?: boolean;
  }) {
    this.term = new Terminal({
      fontSize: opts.fontSize,
      fontFamily: opts.fontFamily,
      fontWeight: 400,
      // Only 400/500/600 are bundled; 700 would be synthesized.
      fontWeightBold: 600,
      lineHeight: 1.0,
      theme: opts.theme,
      cursorBlink: opts.cursorBlink ?? true,
      scrollback: 5000,
      convertEol: false,
      allowProposedApi: true,
      // Option-drag still selects when a TUI has taken the mouse.
      macOptionClickForcesSelection: true,
      linkHandler: { activate: openLink },
    });
    this.fitAddon = new FitAddon();
    this.term.loadAddon(this.fitAddon);
    this.term.loadAddon(new WebLinksAddon(openLink));
    this.dark = isDarkBackground(opts.theme.background);
    // xterm 5.5 has no DEC mode 2031, so a TUI that follows the terminal's theme never hears it flip.
    const parser = this.term.parser;
    parser.registerCsiHandler({ prefix: "?", final: "h" }, (params) =>
      this.noteThemeReports(params, true),
    );
    parser.registerCsiHandler({ prefix: "?", final: "l" }, (params) =>
      this.noteThemeReports(params, false),
    );
    parser.registerCsiHandler({ prefix: "?", final: "n" }, (params) => {
      if (params[0] !== COLOR_SCHEME_QUERY) return false;
      this.reportColorScheme();
      return true;
    });
    // How a TUI that owns the mouse copies, and claude's only way over ssh. Only a write the user
    // just asked for lands: not one replayed on remount, not a program's own. Reads are refused.
    // A frame stream carries the server's copies as `clipboard` events, through the same gate.
    parser.registerOscHandler(CLIPBOARD_OSC, (data) => {
      const payload = data.slice(data.indexOf(";") + 1);
      if (payload && payload !== "?") this.clipboard(payload);
      return true;
    });
  }

  clipboard(data: string): void {
    if (this.userJustActed()) copyFromProgram(data);
  }

  mount(host: HTMLElement): void {
    if (!this.term) return;
    this.term.open(host);
    const noteGesture = () => {
      this.lastGesture = performance.now();
    };
    for (const type of ["keydown", "mousedown", "mouseup"]) {
      this.term.element?.addEventListener(type, noteGesture, true);
    }
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

  bind(terminalId: string): void {
    // Output parsed before the first bind is this pty's; a previous pty's mode is not.
    if (this.terminalId !== undefined && this.terminalId !== terminalId) this.reportsTheme = false;
    this.terminalId = terminalId;
    this.setThemeReports(this.reportsTheme || themeReporting.has(terminalId));
    this.replay = "awaiting";
  }

  write(data: Uint8Array | string): void {
    if (!this.term) return;
    if (this.replay !== "awaiting") {
      this.term.write(data);
      return;
    }
    this.replay = "parsing";
    this.term.write(data, () => {
      this.replay = null;
      // The theme may have flipped while no engine showed this pty.
      if (this.reportsTheme) this.reportColorScheme();
    });
  }

  // A frame carries no replayed copy, so the gesture gate opens at once.
  writeFrame(data: Uint8Array, written: () => void): void {
    if (!this.term) return;
    this.frames = true;
    this.replay = null;
    this.term.write(data, written);
  }

  follow(grid: TerminalGrid): void {
    if (!this.term) return;
    this.following = true;
    // Behind the output already written, which was drawn for the old size.
    this.term.write("", () => {
      this.term?.resize(grid.cols, grid.rows);
      this.measurePane();
      this.showWhole();
    });
  }

  private measurePane(): void {
    try {
      const d = this.fitAddon?.proposeDimensions();
      if (d && d.cols > 0 && d.rows > 0) {
        this.fitted = { cols: d.cols, rows: d.rows };
        this.rendererReady = true;
      }
    } catch {
    }
  }

  // A grid bigger than the pane is scaled down to show whole, without resizing anything.
  private showWhole(): void {
    const el = this.term?.element;
    if (!el || !this.term || !this.fitted) return;
    const scale = Math.min(
      1,
      this.fitted.cols / this.term.cols,
      this.fitted.rows / this.term.rows,
    );
    el.style.transformOrigin = "0 0";
    el.style.transform = scale < 1 ? `scale(${scale})` : "";
  }

  private userJustActed(): boolean {
    return this.replay === null && performance.now() - this.lastGesture < USER_GESTURE_MS;
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
    if (this.following) {
      this.measurePane();
      this.showWhole();
      return this.fitted ?? this.grid();
    }
    try {
      this.fitAddon.fit();
      this.rendererReady = true;
      this.fitRetries = 0;
      this.fitted = this.grid();
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

  async setFontFamily(family: string): Promise<TerminalGrid> {
    this.fontFamilyRequest = family;
    const px = this.term?.options.fontSize ?? 12;
    await document.fonts.load(`${px}px ${family}`).catch(() => undefined);
    if (!this.term || this.fontFamilyRequest !== family) return EMPTY_GRID;
    this.term.options.fontFamily = family;
    return this.fit();
  }

  setTheme(theme: TerminalTheme): void {
    if (!this.term) return;
    this.term.options.theme = theme;
    const dark = isDarkBackground(theme.background);
    if (dark === this.dark) return;
    this.dark = dark;
    // After the recolor, so a program that re-queries OSC 11 reads the new background.
    if (this.reportsTheme) this.reportColorScheme();
  }

  private noteThemeReports(params: (number | number[])[], on: boolean): boolean {
    if (params.includes(THEME_REPORTS_MODE)) this.setThemeReports(on);
    return false;
  }

  private setThemeReports(on: boolean): void {
    this.reportsTheme = on;
    if (!this.terminalId) return;
    if (on) themeReporting.add(this.terminalId);
    else themeReporting.delete(this.terminalId);
  }

  // 1 is dark, 2 is light. A frame stream's program hears the server, in the active desktop's theme.
  private reportColorScheme(): void {
    if (this.frames) return;
    this.term?.input(`\x1b[?997;${this.dark ? 1 : 2}n`, false);
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
    this.setThemeReports(false);
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
    // xterm queues syncScrollArea on a timer at open() and on a frame at reset().
    // Once a resize has recorded the buffer length, a call landing after dispose
    // reads the disposed renderer's dimensions and throws; StrictMode's mount,
    // fit, unmount does exactly that. Verified against @xterm/xterm 5.5.0.
    const viewport = (this.term as unknown as XtermCore | null)?._core?.viewport;
    if (viewport) viewport.syncScrollArea = () => {};
    this.term?.dispose();
    this.term = null;
    this.fitAddon = null;
  }
}

export function createTerminalEngine(opts: {
  fontSize: number;
  fontFamily: string;
  theme: TerminalTheme;
  cursorBlink?: boolean;
}): TerminalEngine {
  return new XtermEngine(opts);
}
