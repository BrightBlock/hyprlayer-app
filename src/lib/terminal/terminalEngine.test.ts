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

class FakeTerminal {
  cols = 80;
  rows = 24;
  options: Record<string, unknown> = {};
  element: HTMLElement | null = null;
  parser = {
    registerCsiHandler: (id: { prefix?: string; final: string }, handler: CsiHandler) => {
      csi.set(`${id.prefix ?? ""}${id.final}`, handler);
      return { dispose() {} };
    },
    registerOscHandler: () => ({ dispose() {} }),
  };
  private selection = "";

  loadAddon(addon: { activate?: (t: FakeTerminal) => void }) {
    addon.activate?.(this);
  }
  open(host: HTMLElement) {
    this.element = host;
  }
  onData() {
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

// Ways showing a followed grid in a pane that doesn't match it could fail:
// - a grid bigger than the pane is clipped, so the watcher can't see the prompt;
// - it is scaled by different factors across and down, distorting it;
// - a grid that fits is scaled, or moved off the top-left corner;
// - the scale goes stale when the pane or the grid changes;
// - a terminal fitted to its own pane is ever scaled.
describe("XtermEngine showing a followed grid", () => {
  function mounted() {
    const host = document.createElement("div");
    const engine = newEngine();
    engine.mount(host);
    return { engine, host };
  }

  function scale(host: HTMLElement): number {
    const m = /^scale\(([\d.]+)\)$/.exec(host.style.transform);
    return m ? Number(m[1]) : 1;
  }

  it("scales a grid bigger than its pane down to show whole, the same both ways", () => {
    const { engine, host } = mounted();
    proposal = { cols: 80, rows: 16 };
    engine.follow({ cols: 104, rows: 31 });
    parseQueued();
    expect(scale(host)).toBeCloseTo(16 / 31);
    expect(host.style.transform).toMatch(/^scale\([\d.]+\)$/);
    expect(["0 0", "0px 0px", "left top"]).toContain(host.style.transformOrigin);
    expect(engine.grid()).toEqual({ cols: 104, rows: 31 });
  });

  it("shows a grid that fits at 1:1", () => {
    const { engine, host } = mounted();
    proposal = { cols: 120, rows: 40 };
    engine.follow({ cols: 104, rows: 31 });
    parseQueued();
    expect(host.style.transform).toBe("");
  });

  it("rescales when the pane changes", () => {
    const { engine, host } = mounted();
    proposal = { cols: 52, rows: 31 };
    engine.follow({ cols: 104, rows: 31 });
    parseQueued();
    expect(scale(host)).toBeCloseTo(0.5);
    proposal = { cols: 120, rows: 40 };
    engine.fit();
    expect(host.style.transform).toBe("");
  });

  it("rescales once the grid changes, not before", () => {
    const { engine, host } = mounted();
    proposal = { cols: 80, rows: 24 };
    engine.follow({ cols: 160, rows: 24 });
    parseQueued();
    expect(scale(host)).toBeCloseTo(0.5);
    engine.follow({ cols: 80, rows: 24 });
    expect(scale(host)).toBeCloseTo(0.5);
    parseQueued();
    expect(host.style.transform).toBe("");
  });

  it("never scales a terminal fitted to its own pane", () => {
    const { engine, host } = mounted();
    proposal = { cols: 90, rows: 20 };
    engine.fit();
    expect(host.style.transform).toBe("");
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
