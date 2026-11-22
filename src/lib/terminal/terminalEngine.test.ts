// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";

const writes: string[] = [];
const disposeCalls: string[] = [];
// What the terminal did, in the order it did it: xterm parses a write later, a resize at once.
const parsed: string[] = [];
let queued: { data: string; cb?: () => void }[] = [];
let proposal: { cols: number; rows: number } | undefined;
// What the terminal sent the program, and what reached the clipboard.
const inputs: string[] = [];
const copied: string[] = [];
type CsiHandler = (params: (number | number[])[]) => boolean;
const csi = new Map<string, CsiHandler>();

// xterm runs a write's callback once that write is parsed, before the next one.
function parseQueued() {
  const batch = queued;
  queued = [];
  for (const { data, cb } of batch) {
    if (data) parsed.push(`parsed:${data}`);
    cb?.();
  }
}

const terminals: FakeTerminal[] = [];

class FakeTerminal {
  cols = 80;
  rows = 24;
  options: Record<string, unknown> = {};
  element: HTMLElement | null = null;
  buffer = { active: { cursorX: 0, cursorY: 0 } };
  _core = { _renderService: { dimensions: { css: { cell: { width: 10, height: 20 } } } } };
  private cursorMoved: (() => void)[] = [];
  private typed: ((data: string) => void)[] = [];
  parser = {
    registerCsiHandler: (id: { prefix?: string; final: string }, handler: CsiHandler) => {
      csi.set(`${id.prefix ?? ""}${id.final}`, handler);
      return { dispose() {} };
    },
    registerOscHandler: () => ({ dispose() {} }),
  };
  private selection = "";

  constructor() {
    terminals.push(this);
  }
  moveCursor(x: number, y: number) {
    this.buffer.active = { cursorX: x, cursorY: y };
    this.cursorMoved.forEach((cb) => cb());
  }
  type(data: string) {
    this.typed.forEach((cb) => cb(data));
  }
  onCursorMove(cb: () => void) {
    this.cursorMoved.push(cb);
    return { dispose() {} };
  }
  loadAddon(addon: { activate?: (t: FakeTerminal) => void }) {
    addon.activate?.(this);
  }
  open(host: HTMLElement) {
    this.element = host;
  }
  onData(cb: (data: string) => void) {
    this.typed.push(cb);
    return { dispose: () => disposeCalls.push("onData") };
  }
  onKey() {
    return { dispose: () => disposeCalls.push("onKey") };
  }
  write(data: Uint8Array | string, cb?: () => void) {
    const text = typeof data === "string" ? data : new TextDecoder().decode(data);
    writes.push(text);
    queued.push({ data: text, cb });
  }
  resize(cols: number, rows: number) {
    this.cols = cols;
    this.rows = rows;
    parsed.push(`resize:${cols}x${rows}`);
  }
  refresh() {}
  focus() {}
  hasSelection() {
    return this.selection.length > 0;
  }
  getSelection() {
    return this.selection;
  }
  paste() {}
  input(data: string) {
    inputs.push(data);
  }
  attachCustomKeyEventHandler() {}
  reset() {}
  dispose() {
    disposeCalls.push("terminal");
  }
}

class FakeFitAddon {
  terminal: FakeTerminal | null = null;
  activate(t: FakeTerminal) {
    this.terminal = t;
  }
  proposeDimensions() {
    return proposal;
  }
  fit() {
    if (proposal) this.terminal?.resize(proposal.cols, proposal.rows);
  }
  dispose() {}
}

const fitAddons: FakeFitAddon[] = [];

vi.mock("@xterm/xterm", () => ({ Terminal: FakeTerminal }));
vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    constructor() {
      const a = new FakeFitAddon();
      fitAddons.push(a);
      return a as unknown as this;
    }
  },
}));
vi.mock("@xterm/addon-web-links", () => ({
  WebLinksAddon: class {
    dispose() {}
  },
}));
vi.mock("@xterm/addon-webgl", () => ({
  WebglAddon: class {
    onContextLoss() {}
    dispose() {}
  },
}));
vi.mock("@xterm/xterm/css/xterm.css", () => ({}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  writeText: async (text: string) => {
    copied.push(text);
  },
}));

const { createTerminalEngine } = await import("./terminalEngine");

function newEngine() {
  return createTerminalEngine({ fontSize: 12, fontFamily: "monospace", theme: {} });
}

// The fix is that dispose() drops the engine's Terminal and FitAddon references,
// so reach into them directly — that release is the contract under test, not an
// incidental detail.
function refs(engine: unknown) {
  const e = engine as { term: unknown; fitAddon: unknown };
  return { term: e.term, fitAddon: e.fitAddon };
}

beforeEach(() => {
  writes.length = 0;
  disposeCalls.length = 0;
  fitAddons.length = 0;
  terminals.length = 0;
  parsed.length = 0;
  queued = [];
  proposal = undefined;
  inputs.length = 0;
  copied.length = 0;
});

describe("XtermEngine disposal", () => {

  it("stops forwarding writes once disposed", () => {
    const engine = newEngine();
    engine.write("before");
    engine.dispose();
    engine.write("after");
    engine.write(new TextEncoder().encode("also after"));
    expect(writes).toEqual(["before"]);
  });

  it("releases the terminal and the fit addon on dispose", () => {
    const engine = newEngine();
    expect(refs(engine).term).toBeInstanceOf(FakeTerminal);
    expect(refs(engine).fitAddon).not.toBeNull();

    engine.dispose();

    expect(refs(engine).term).toBeNull();
    expect(refs(engine).fitAddon).toBeNull();
    expect(disposeCalls).toContain("terminal");
  });

  it("survives every interface call after dispose", () => {
    const engine = newEngine();
    engine.dispose();
    expect(() => {
      engine.mount(document.createElement("div"));
      engine.write("x");
      engine.refresh();
      engine.setTheme({});
      engine.focus();
      engine.paste("x");
      engine.interceptKeys(() => true);
      engine.reset();
    }).not.toThrow();
    expect(engine.grid()).toEqual({ cols: 0, rows: 0 });
    expect(engine.fit()).toEqual({ cols: 0, rows: 0 });
    expect(engine.setFontSize(20)).toEqual({ cols: 0, rows: 0 });
    expect(engine.hasSelection()).toBe(false);
    expect(engine.selectionText()).toBe("");
  });

  it("returns a no-op unsubscribe when subscribing after dispose", () => {
    const engine = newEngine();
    engine.dispose();
    expect(() => {
      engine.onInput(() => {})();
      engine.onKey(() => {})();
    }).not.toThrow();
    expect(disposeCalls).not.toContain("onData");
  });

  it("still unsubscribes listeners registered while live", () => {
    const engine = newEngine();
    const offInput = engine.onInput(() => {});
    const offKey = engine.onKey(() => {});
    offInput();
    offKey();
    expect(disposeCalls).toEqual(expect.arrayContaining(["onData", "onKey"]));
  });

  it("is idempotent", () => {
    const engine = newEngine();
    engine.dispose();
    expect(() => engine.dispose()).not.toThrow();
    expect(disposeCalls.filter((c) => c === "terminal")).toHaveLength(1);
  });
});

// Ways following the pty's size could fail:
// - the size lands before output written for the old size is parsed;
// - output written after the size is parsed before the size lands;
// - a fit still resizes a following terminal to its host, so two desktops fight over the size;
// - a fit reports the followed size instead of what would fit the host;
// - before any size arrives (an older server), the terminal stops fitting itself.
describe("XtermEngine following the pty's size", () => {
  it("applies a size between the output written before and after it", () => {
    const engine = newEngine();
    engine.write("old");
    engine.follow({ cols: 100, rows: 30 });
    engine.write("new");
    expect(parsed).toEqual([]);
    parseQueued();
    expect(parsed).toEqual(["parsed:old", "resize:100x30", "parsed:new"]);
  });

  it("reports the host's fit without resizing a following terminal", () => {
    const engine = newEngine();
    engine.follow({ cols: 100, rows: 30 });
    parseQueued();
    proposal = { cols: 90, rows: 20 };
    expect(engine.fit()).toEqual({ cols: 90, rows: 20 });
    expect(engine.grid()).toEqual({ cols: 100, rows: 30 });
  });

  it("fits itself to its host until it learns the pty's size", () => {
    const engine = newEngine();
    proposal = { cols: 90, rows: 20 };
    expect(engine.fit()).toEqual({ cols: 90, rows: 20 });
    expect(engine.grid()).toEqual({ cols: 90, rows: 20 });
  });
});

// Ways showing another desktop's grid in a pane that doesn't match it could fail:
// - it is scaled, so its text is unreadable at a small pane's size;
// - a grid bigger than the pane is clipped with no way to scroll to the rest;
// - the program's cursor scrolls out of sight while the user isn't looking elsewhere;
// - the view drags the user back to the cursor while they read another part of the grid;
// - it never goes back to the cursor once they scroll back to it or type;
// - a grid that fits, or a terminal fitted to its own pane, scrolls or moves off the top left.
describe("XtermEngine showing a followed grid", () => {
  const CELL = { width: 10, height: 20 };

  function mounted(pane: { width: number; height: number }) {
    const host = document.createElement("div");
    Object.defineProperty(host, "clientWidth", { value: pane.width });
    Object.defineProperty(host, "clientHeight", { value: pane.height });
    Object.defineProperty(host, "scrollLeft", { value: 0, writable: true });
    Object.defineProperty(host, "scrollTop", { value: 0, writable: true });
    const engine = newEngine();
    engine.mount(host);
    proposal = { cols: pane.width / CELL.width, rows: pane.height / CELL.height };
    return { engine, host, term: terminals[terminals.length - 1] };
  }

  const scroll = (host: HTMLElement) => [host.scrollLeft, host.scrollTop];

  it("shows a grid bigger than its pane at 1:1, scrollable both ways", () => {
    const { engine, host } = mounted({ width: 800, height: 320 });
    engine.follow({ cols: 104, rows: 31 });
    parseQueued();
    expect(host.style.transform).toBe("");
    expect(host.style.overflow).toBe("auto");
    expect(engine.grid()).toEqual({ cols: 104, rows: 31 });
  });

  it("keeps the program's cursor in view as it moves", () => {
    const { engine, host, term } = mounted({ width: 800, height: 320 });
    engine.follow({ cols: 104, rows: 31 });
    parseQueued();
    term.moveCursor(100, 30);
    expect(scroll(host)).toEqual([1010 - 800, 620 - 320]);
    term.moveCursor(2, 0);
    expect(scroll(host)).toEqual([20, 0]);
  });

  it("leaves the view where the user scrolled it, until they scroll back to the cursor", () => {
    const { engine, host, term } = mounted({ width: 800, height: 320 });
    engine.follow({ cols: 104, rows: 31 });
    parseQueued();
    term.moveCursor(100, 30);
    host.scrollLeft = 0;
    host.scrollTop = 0;
    host.dispatchEvent(new Event("scroll"));
    term.moveCursor(101, 30);
    expect(scroll(host)).toEqual([0, 0]);
    host.scrollLeft = 300;
    host.scrollTop = 300;
    host.dispatchEvent(new Event("scroll"));
    term.moveCursor(2, 30);
    expect(scroll(host)).toEqual([20, 300]);
  });

  it("keeps following when its own scroll's event lands after the cursor moved on", () => {
    const { engine, host, term } = mounted({ width: 800, height: 320 });
    engine.follow({ cols: 104, rows: 31 });
    parseQueued();
    term.moveCursor(100, 30);
    term.buffer.active = { cursorX: 2, cursorY: 0 };
    host.dispatchEvent(new Event("scroll"));
    term.moveCursor(2, 0);
    expect(scroll(host)).toEqual([20, 0]);
  });

  it("goes back to the cursor on the next input", () => {
    const { engine, host, term } = mounted({ width: 800, height: 320 });
    engine.follow({ cols: 104, rows: 31 });
    parseQueued();
    term.moveCursor(100, 30);
    host.scrollLeft = 0;
    host.scrollTop = 0;
    host.dispatchEvent(new Event("scroll"));
    term.type("j");
    expect(scroll(host)).toEqual([1010 - 800, 620 - 320]);
  });

  it("shows a grid that fits at 1:1 from the top left, with no scrolling", () => {
    const { engine, host, term } = mounted({ width: 1200, height: 800 });
    engine.follow({ cols: 104, rows: 31 });
    parseQueued();
    term.moveCursor(100, 30);
    expect(host.style.transform).toBe("");
    expect(host.style.overflow).toBe("");
    expect(scroll(host)).toEqual([0, 0]);
  });

  it("stops scrolling once the grid fits the pane", () => {
    const { engine, host, term } = mounted({ width: 800, height: 320 });
    engine.follow({ cols: 104, rows: 31 });
    parseQueued();
    term.moveCursor(100, 30);
    engine.follow({ cols: 80, rows: 16 });
    parseQueued();
    expect(host.style.overflow).toBe("");
    expect(scroll(host)).toEqual([0, 0]);
  });

  it("never scrolls a terminal fitted to its own pane", () => {
    const { engine, host, term } = mounted({ width: 900, height: 400 });
    engine.fit();
    term.moveCursor(89, 19);
    expect(host.style.overflow).toBe("");
    expect(scroll(host)).toEqual([0, 0]);
  });
});

describe("XtermEngine on a stream of the server's frames", () => {
  const darkEngine = () =>
    createTerminalEngine({ fontSize: 12, fontFamily: "monospace", theme: { background: "#000000" } });

  it("copies what the program copied only right after a user gesture", () => {
    const host = document.createElement("div");
    const engine = newEngine();
    engine.mount(host);
    engine.bind("t1");
    engine.writeFrame(new TextEncoder().encode("frame"), () => {});
    engine.clipboard(btoa("unasked"));
    host.dispatchEvent(new MouseEvent("mousedown"));
    engine.clipboard(btoa("asked"));
    expect(copied).toEqual(["asked"]);
  });

  it("leaves the program's theme reports to the server", () => {
    const raw = darkEngine();
    csi.get("?h")!([2031]);
    raw.setTheme({ background: "#ffffff" });
    expect(inputs).toEqual(["\x1b[?997;2n"]);

    inputs.length = 0;
    const framed = darkEngine();
    framed.writeFrame(new TextEncoder().encode("frame"), () => {});
    csi.get("?h")!([2031]);
    framed.setTheme({ background: "#ffffff" });
    expect(inputs).toEqual([]);
  });
});
