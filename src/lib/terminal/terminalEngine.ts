import { Terminal, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import "@xterm/xterm/css/xterm.css";


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

class XtermEngine implements TerminalEngine {
  private readonly term: Terminal;
  private readonly fitAddon: FitAddon;
  private webgl?: WebglAddon;
  private rendererReady = false;

  constructor(opts: { fontSize: number; theme: TerminalTheme; cursorBlink?: boolean }) {
    this.term = new Terminal({
      fontSize: opts.fontSize,
      fontFamily:
        '"JetBrains Mono Variable", ui-monospace, SFMono-Regular, Menlo, monospace',
      fontWeight: 400,
      fontWeightBold: 700,
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
    this.term.open(host);
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => {
        try {
          webgl.dispose();
        } catch {
        }
        if (this.webgl === webgl) this.webgl = undefined;
      });
      this.term.loadAddon(webgl);
      this.webgl = webgl;
    } catch {
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
    // xterm computes renderer dimensions on its first paint.
    try {
      this.fitAddon.fit();
      this.rendererReady = true;
    } catch {
    }
    return this.grid();
  }

  grid(): TerminalGrid {
    return { cols: this.term.cols, rows: this.term.rows };
  }

  refresh(): void {
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
    // xterm 5.5 may throw while reverting the WebGL renderer.
    try {
      this.webgl?.dispose();
    } catch {
    }
    this.webgl = undefined;
    this.term.dispose();
  }
}

export function createTerminalEngine(opts: {
  fontSize: number;
  theme: TerminalTheme;
  cursorBlink?: boolean;
}): TerminalEngine {
  return new XtermEngine(opts);
}
