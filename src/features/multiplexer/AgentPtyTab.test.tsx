// @vitest-environment jsdom
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => {
  class FakeChannel {
    onmessage: (event: unknown) => void = () => {};
  }
  // As xterm 5.5 orders it: `write` queues and a full reset (`ESC c`) in the queue clears in turn.
  class FakeScreen {
    shown = "";
    queued: string[] = [];
    write(data: Uint8Array | string) {
      this.queued.push(typeof data === "string" ? data : new TextDecoder().decode(data));
    }
    writeFrame(data: Uint8Array) {
      this.write(data);
    }
    clipboard() {}
    parse() {
      for (const data of this.queued.splice(0)) {
        const ris = data.lastIndexOf("\x1bc");
        this.shown = ris >= 0 ? data.slice(ris + 2) : this.shown + data;
      }
    }
    reset() {
      this.shown = "";
    }
    mount() {}
    fit() {
      return { ...grid };
    }
    focus() {}
    bind() {}
    refresh() {}
    park() {}
    setTheme() {}
    setFontSize() {
      return this.fit();
    }
    dispose() {}
    onInput() {
      return () => {};
    }
    onKey() {
      return () => {};
    }
  }
  const grid = { cols: 80, rows: 24 };
  const observers: Array<() => void> = [];
  const attaches: Array<{ id: string; offset: number | null; ch: FakeChannel }> = [];
  const reconnects: Array<() => void> = [];
  const launches = ["t1", "t2", "t3", "t4"];
  const ok = <T,>(data: T) => ({ status: "ok" as const, data });
  const controls: { onRestart?: (keep: object) => void } = {};
  const down = {
    status: "error" as const,
    error: { code: "server_unavailable", message: "The server is not connected.", remedy: null },
  };
  return {
    FakeChannel,
    FakeScreen,
    grid,
    observers,
    attaches,
    reconnects,
    launches,
    ok,
    down,
    controls,
    back: [] as Array<(terminalId: string) => void>,
    serverUp: true,
    launchFails: false,
    ensures: 0,
    screen: undefined as FakeScreen | undefined,
  };
});

vi.mock("@tauri-apps/api/core", () => ({ Channel: h.FakeChannel }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ onFocusChanged: async () => () => {} }),
}));
vi.mock("@/lib/bindings", () => ({
  commands: {
    terminalAttach: vi.fn(async (id: string, offset: number | null, ch: InstanceType<typeof h.FakeChannel>) => {
      h.attaches.push({ id, offset, ch });
      return h.serverUp ? h.ok(true) : h.down;
    }),
    agentResize: vi.fn(async () => h.ok(null)),
    terminalResize: vi.fn(async () => h.ok(null)),
    terminalFocus: vi.fn(async () => h.ok(null)),
    terminalRelease: vi.fn(async () => h.ok(null)),
    terminalFrameAck: vi.fn(async () => h.ok(null)),
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
vi.mock("@/lib/toast", () => ({ useToast: { getState: () => ({ show: () => {} }) } }));
vi.mock("@/features/session/scriptsStore", () => ({
  useScriptsStore: (select: (s: { dock: object }) => unknown) => select({ dock: {} }),
}));
vi.mock("./agentRuntime", () => ({
  ensureAgent: vi.fn(async () => {
    h.ensures += 1;
    return h.launchFails ? null : { terminalId: h.launches.shift() };
  }),
  closeAgent: vi.fn(),
  onAgentTerminalGone: () => () => {},
  onAgentTerminalBack: (_ws: string, _shell: number, fn: (terminalId: string) => void) => {
    h.back.push(fn);
    return () => h.back.splice(h.back.indexOf(fn), 1);
  },
  resolveResume: vi.fn(async () => false),
  setAgentBypass: vi.fn(),
}));
vi.mock("./engines", () => ({ engineMintsViaExec: () => false, engineResumesById: () => false }));
vi.mock("./multiplexerStore", () => ({ markAgentStarted: vi.fn(), useAgentState: () => undefined }));
vi.mock("./PtySessionControls", () => ({
  PtySessionControls: (props: { onRestart: (keep: object) => void }) => {
    h.controls.onRestart = props.onRestart;
    return null;
  },
}));

globalThis.ResizeObserver = class {
  constructor(private cb: () => void) {}
  observe() {
    h.observers.push(this.cb);
  }
  disconnect() {
    h.observers.splice(h.observers.indexOf(this.cb), 1);
  }
} as unknown as typeof ResizeObserver;
Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => 800 });
Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => 600 });
globalThis.requestAnimationFrame = ((cb: FrameRequestCallback) => setTimeout(() => cb(0), 0)) as typeof requestAnimationFrame;

const { AgentPtyTab } = await import("./AgentPtyTab");
const { commands } = await import("@/lib/bindings");

const output = (offset: number, text: string) => ({ kind: "output", data: btoa(text), offset });
const frame = (seq: number, history: number, text: string) => ({
  kind: "output",
  data: btoa(text),
  offset: history,
  frame: seq,
});
const settle = () => act(() => new Promise((r) => setTimeout(r, 10)));

function mount(terminalId: string, autoFocus?: boolean) {
  return render(tab(terminalId, autoFocus));
}

function tab(terminalId: string, autoFocus?: boolean) {
  return (
    <AgentPtyTab
      workstreamId="ws"
      shellId={1}
      engine={{ id: "claude-code", label: "Claude" } as never}
      projectPath="/p"
      workstreamTitle="t"
      terminalId={terminalId}
      autoFocus={autoFocus}
      onTerminalId={() => {}}
      notifyOnBlocked={false}
    />
  );
}

/** The pane is dragged through `sizes`, a frame each, ending on the last. */
async function drag(...sizes: Array<[number, number]>) {
  for (const [cols, rows] of sizes) {
    Object.assign(h.grid, { cols, rows });
    await act(async () => h.observers.forEach((fn) => fn()));
    await act(() => new Promise((r) => setTimeout(r, 5)));
  }
}

afterEach(() => {
  cleanup();
  Object.assign(h.grid, { cols: 80, rows: 24 });
  vi.mocked(commands.agentResize).mockClear();
  vi.mocked(commands.terminalResize).mockClear();
  h.serverUp = true;
  h.launchFails = false;
  h.ensures = 0;
});

describe("an agent tab's restart", () => {
  it("leaves the new agent's screen and resume point to the new agent", async () => {
    mount("t1");
    await settle();
    const old = h.attaches[h.attaches.length - 1];
    old.ch.onmessage(output(0, "old agent "));
    h.screen!.parse();

    await act(async () => h.controls.onRestart?.({}));
    await settle();
    const fresh = h.attaches[h.attaches.length - 1];
    fresh.ch.onmessage(output(0, "\x1b[?1000hnew agent"));
    old.ch.onmessage(output(10, "old tail"));
    fresh.ch.onmessage(output(19, " ready"));
    h.screen!.parse();

    expect(fresh.id).toBe("t2");
    expect(h.screen!.shown).toBe("\x1b[?1000hnew agent ready");

    await act(async () => h.reconnects.forEach((fn) => fn()));
    await settle();
    const resumed = h.attaches[h.attaches.length - 1];
    expect([resumed.id, resumed.offset]).toEqual(["t2", 25]);
  });

  it("resumes the new agent's frames from that agent's history count", async () => {
    mount("t3");
    await settle();
    const old = h.attaches[h.attaches.length - 1];
    old.ch.onmessage(frame(1, 300, "old agent "));
    h.screen!.parse();

    await act(async () => h.controls.onRestart?.({}));
    await settle();
    const fresh = h.attaches[h.attaches.length - 1];
    fresh.ch.onmessage(frame(1, 0, "\x1bcnew agent"));
    old.ch.onmessage(frame(2, 310, "old tail"));
    fresh.ch.onmessage(frame(2, 4, " ready"));
    h.screen!.parse();

    expect(fresh.id).toBe("t4");
    expect(h.screen!.shown).toBe("new agent ready");

    await act(async () => h.reconnects.forEach((fn) => fn()));
    await settle();
    const resumed = h.attaches[h.attaches.length - 1];
    expect([resumed.id, resumed.offset]).toEqual(["t4", 4]);
  });
});

describe("an agent tab shown again", () => {
  it("shows its last screen at once and picks its frames up where it left off", async () => {
    h.launches.push("t5", "t5");
    mount("t5");
    await settle();
    h.attaches[h.attaches.length - 1].ch.onmessage(frame(1, 120, "\x1bcthe agent's screen"));
    h.screen!.parse();

    cleanup();
    mount("t5");
    expect(h.screen!.shown).toBe("the agent's screen");

    await settle();
    const resumed = h.attaches[h.attaches.length - 1];
    expect([resumed.id, resumed.offset]).toEqual(["t5", 120]);
  });
});

describe("an agent tab shown while the server is down", () => {
  it("attaches once the server reconnects", async () => {
    h.serverUp = false;
    h.launches.push("t6");
    mount("t6");
    await settle();
    expect(h.attaches[h.attaches.length - 1]?.id).toBe("t6");
    const before = h.attaches.length;

    h.serverUp = true;
    await act(async () => h.reconnects.forEach((fn) => fn()));
    await settle();

    expect(h.attaches.slice(before).map((a) => a.id)).toEqual(["t6"]);
  });

  it("explains a failed launch and retries it on reconnect", async () => {
    h.launchFails = true;
    mount("t7");
    await settle();
    h.screen!.parse();
    expect(h.ensures).toBe(1);
    expect(h.screen!.shown).toContain("The agent didn't start. It tries again when the server reconnects.");

    h.launchFails = false;
    h.launches.push("t7");
    await act(async () => h.reconnects.forEach((fn) => fn()));
    await settle();

    expect(h.ensures).toBe(2);
    expect(h.attaches[h.attaches.length - 1]?.id).toBe("t7");
  });
});

describe("an agent tab whose agent a restarted server resumed", () => {
  it("starts its screen over and attaches the new process from its first byte", async () => {
    h.launches.push("t8");
    mount("t8");
    await settle();
    h.attaches[h.attaches.length - 1].ch.onmessage(output(0, "before the restart"));
    h.screen!.parse();

    await act(async () => h.back.forEach((fn) => fn("t8")));
    await settle();
    const resumed = h.attaches[h.attaches.length - 1];
    resumed.ch.onmessage(output(0, "resumed"));
    h.screen!.parse();

    expect([resumed.id, resumed.offset]).toEqual(["t8", null]);
    expect(h.screen!.shown).toBe("resumed");
  });

  it("ignores another terminal coming back", async () => {
    h.launches.push("t9");
    mount("t9");
    await settle();
    const before = h.attaches.length;
    await act(async () => h.back.forEach((fn) => fn("someone-else")));
    await settle();
    expect(h.attaches.length).toBe(before);
  });
});

describe("an agent tab's size", () => {
  const sizesSent = () => [
    ...vi.mocked(commands.agentResize).mock.calls,
    ...vi.mocked(commands.terminalResize).mock.calls,
  ];

  it("is left alone by a pane that isn't focused, however that pane changes", async () => {
    h.launches.push("t10");
    mount("t10", false);
    await settle();
    await drag([100, 30], [60, 20]);
    await act(() => new Promise((r) => setTimeout(r, 200)));
    expect(sizesSent()).toEqual([]);
  });

  it("follows the focused pane once a drag settles, at the size it ends on", async () => {
    h.launches.push("t11");
    mount("t11", true);
    await settle();
    expect(vi.mocked(commands.agentResize).mock.calls).toEqual([["t11", 80, 24]]);
    await drag([90, 24], [100, 26], [110, 28]);
    expect(vi.mocked(commands.terminalResize).mock.calls).toEqual([]);
    await act(() => new Promise((r) => setTimeout(r, 200)));
    expect(vi.mocked(commands.terminalResize).mock.calls).toEqual([["t11", 110, 28]]);
  });

  it("moves to a pane when it becomes the focused one", async () => {
    h.launches.push("t12");
    const { rerender } = mount("t12", false);
    await settle();
    await drag([120, 40]);
    rerender(tab("t12", true));
    await settle();
    expect(sizesSent()).toEqual([["t12", 120, 40]]);
  });
});
