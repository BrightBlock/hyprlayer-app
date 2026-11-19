// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";

const writes: string[] = [];
const disposeCalls: string[] = [];

class FakeTerminal {
  cols = 80;
  rows = 24;
  options: Record<string, unknown> = {};
  element: HTMLElement | null = null;
  parser = {
    registerCsiHandler: () => ({ dispose() {} }),
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
  write(data: Uint8Array | string) {
    writes.push(typeof data === "string" ? data : new TextDecoder().decode(data));
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
  input() {}
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
  fit() {}
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
