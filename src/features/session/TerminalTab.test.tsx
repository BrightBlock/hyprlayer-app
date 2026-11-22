// @vitest-environment jsdom
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => {
  class FakeChannel {
    onmessage: (event: unknown) => void = () => {};
  }
  // As xterm 5.5 orders it: `write` queues, `reset()` clears only what is already parsed,
  // and a full reset (`ESC c`) in the queue clears in turn.
  class FakeScreen {
    shown = "";
    queued: Array<{ data: string; written?: () => void }> = [];
    resets = 0;
    write(data: Uint8Array | string) {
      this.queued.push({ data: typeof data === "string" ? data : new TextDecoder().decode(data) });
    }
    writeFrame(data: Uint8Array, written: () => void) {
      this.queued.push({ data: new TextDecoder().decode(data), written });
    }
    clipboard() {}
    parse() {
      for (const { data, written } of this.queued.splice(0)) {
        const ris = data.lastIndexOf("\x1bc");
        this.shown = ris >= 0 ? data.slice(ris + 2) : this.shown + data;
        written?.();
      }
    }
    reset() {
      this.shown = "";
      this.resets += 1;
    }
    mount() {}
    fit() {
      return { cols: 80, rows: 24 };
    }
    focus() {}
    bind() {}
    refresh() {}
    dispose() {}
    onInput() {
      return () => {};
    }
  }
  const attaches: Array<{ id: string; offset: number | null; ch: FakeChannel }> = [];
  const reconnects: Array<() => void> = [];
  const acks: number[] = [];
  const ok = <T,>(data: T) => ({ status: "ok" as const, data });
  return { FakeChannel, FakeScreen, attaches, reconnects, acks, ok, screen: undefined as FakeScreen | undefined };
});

vi.mock("@tauri-apps/api/core", () => ({ Channel: h.FakeChannel }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ onFocusChanged: async () => () => {} }),
}));
vi.mock("@/lib/bindings", () => ({
  commands: {
    terminalAttach: vi.fn(async (id: string, offset: number | null, ch: InstanceType<typeof h.FakeChannel>) => {
      h.attaches.push({ id, offset, ch });
      return h.ok(true);
    }),
    terminalResize: vi.fn(async () => h.ok(null)),
    terminalRelease: vi.fn(async () => h.ok(null)),
    terminalClose: vi.fn(async () => h.ok(null)),
    terminalFrameAck: vi.fn(async (_channel: number, seq: number) => {
      h.acks.push(seq);
      return h.ok(null);
    }),
  },
}));
vi.mock("@/lib/events", () => ({
  onServerReconnected: (fn: () => void) => {
    h.reconnects.push(fn);
    return () => h.reconnects.splice(h.reconnects.indexOf(fn), 1);
  },
}));
vi.mock("@/lib/terminal/terminalEngine", () => ({
  createTerminalEngine: () => (h.screen = new h.FakeScreen()),
}));
vi.mock("@/lib/terminal/terminalChrome", () => ({
  attachTerminalClipboard: () => {},
  useTerminalThemeSync: () => {},
  useTerminalZoom: () => 13,
}));
vi.mock("@/lib/terminal/terminalTheme", () => ({ xtermThemeFromCss: () => ({}) }));
vi.mock("@/lib/appearance", () => ({ monoFontStack: () => "mono" }));
vi.mock("@/lib/log", () => ({ log: { warn: () => {}, error: () => {}, info: () => {}, debug: () => {} } }));
vi.mock("@/features/hosts/serverAutoUpdate", () => ({ noteActivity: () => {} }));

globalThis.ResizeObserver = class {
  observe() {}
  disconnect() {}
} as unknown as typeof ResizeObserver;

const { TerminalTab } = await import("./TerminalTab");

const output = (offset: number, text: string) => ({ kind: "output", data: btoa(text), offset });
const frame = (seq: number, history: number, text: string) => ({
  kind: "output",
  data: btoa(text),
  offset: history,
  frame: seq,
});
const settle = () => act(() => new Promise((r) => setTimeout(r, 0)));

async function mounted() {
  render(<TerminalTab workstreamId="ws" shellId={1} terminalId="t1" onTerminalId={() => {}} />);
  await settle();
  return h.attaches[h.attaches.length - 1];
}

beforeEach(() => {
  h.attaches.length = 0;
  h.reconnects.length = 0;
  h.acks.length = 0;
});
afterEach(cleanup);

describe("a terminal tab's output", () => {
  it("keeps the screen when a live stream skips output the server trimmed", async () => {
    const { ch } = await mounted();
    ch.onmessage(output(0, "prompt$ "));
    h.screen!.parse();
    ch.onmessage(output(5000, "later"));
    h.screen!.parse();

    expect(h.screen!.resets).toBe(0);
    expect(h.screen!.shown).toBe("prompt$ later");
  });

  it("resumes after the last byte shown once the server reconnects", async () => {
    const { ch } = await mounted();
    ch.onmessage(output(0, "abc"));
    h.screen!.parse();

    await act(async () => h.reconnects.forEach((fn) => fn()));
    await settle();
    const resumed = h.attaches[h.attaches.length - 1];
    resumed.ch.onmessage(output(3, "def"));
    h.screen!.parse();

    expect(resumed.offset).toBe(3);
    expect(h.screen!.shown).toBe("abcdef");
  });

  it("starts over behind what the screen has queued when a resume doesn't hold", async () => {
    const { ch } = await mounted();
    ch.onmessage(output(0, "stale"));

    await act(async () => h.reconnects.forEach((fn) => fn()));
    await settle();
    const resumed = h.attaches[h.attaches.length - 1];
    resumed.ch.onmessage(output(9000, "replay"));
    h.screen!.parse();

    expect(resumed.offset).toBe(5);
    expect(h.screen!.shown).toBe("replay");
  });
});

describe("a terminal tab rendering the server's frames", () => {
  it("acks each frame once the screen has drawn it", async () => {
    const { ch } = await mounted();
    ch.onmessage(frame(1, 0, "one"));
    ch.onmessage(frame(2, 0, "two"));
    await settle();
    expect(h.acks).toEqual([]);

    h.screen!.parse();
    await settle();
    expect(h.acks).toEqual([1, 2]);
  });

  it("resumes from its history count once the server reconnects, keeping the screen", async () => {
    const { ch } = await mounted();
    ch.onmessage(frame(1, 0, "prompt$ "));
    ch.onmessage(frame(2, 12, "ls"));
    h.screen!.parse();

    await act(async () => h.reconnects.forEach((fn) => fn()));
    await settle();
    const resumed = h.attaches[h.attaches.length - 1];
    resumed.ch.onmessage(frame(1, 15, " done"));
    h.screen!.parse();

    expect(resumed.offset).toBe(12);
    expect(h.screen!.resets).toBe(0);
    expect(h.screen!.shown).toBe("prompt$ ls done");
  });
});
