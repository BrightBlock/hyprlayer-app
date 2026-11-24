// @vitest-environment jsdom
import { beforeAll, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => {
  let nextChannel = 1;
  class FakeChannel {
    id = nextChannel++;
    onmessage: (event: unknown) => void = () => {};
  }
  type Printed = { offset: number; data: string };
  type Terminal = { workstreamId: string; hooked: boolean; streams: Set<FakeChannel>; printed: Printed[] };
  const terminal = (workstreamId: string, hooked: boolean, streams: Set<FakeChannel>): Terminal => ({
    workstreamId,
    hooked,
    streams,
    printed: [],
  });
  const send = (ch: FakeChannel, p: Printed) => {
    ch.onmessage({ kind: "output", ...p });
    server.sent += p.data.length;
  };
  // The server's side: live terminals, the channels each still streams to, and what it sent.
  const server = {
    terminals: new Map<string, Terminal>(),
    launches: 0,
    nextHooked: true,
    attaches: [] as string[],
    sent: 0,
    listeners: [] as Array<(e: { payload: unknown }) => void>,
  };
  const ok = <T>(data: T) => ({ status: "ok" as const, data });
  const agent = (terminalId: string, state: string) => ({
    terminalId,
    workstreamId: server.terminals.get(terminalId)?.workstreamId ?? "",
    engineId: "claude-code",
    state,
    providerSessionId: null,
    activity: null,
    lastReply: null,
  });
  const commands = {
    ptyLaunch: vi.fn(async (workstreamId: string, ...rest: unknown[]) => {
      const terminalId = `t${++server.launches}`;
      const ch = rest[rest.length - 1] as FakeChannel;
      server.terminals.set(terminalId, terminal(workstreamId, server.nextHooked, new Set([ch])));
      return ok({ terminalId, hooked: server.nextHooked, providerSessionId: null });
    }),
    // Replays what follows `offset`, or everything without one, as the server's ring does.
    terminalAttach: vi.fn(async (terminalId: string, ...rest: unknown[]) => {
      const [offset, ch] = (rest.length === 1 ? [null, rest[0]] : rest) as [number | null, FakeChannel];
      server.attaches.push(terminalId);
      const t = server.terminals.get(terminalId);
      for (const p of t?.printed ?? []) if (offset == null || p.offset >= offset) send(ch, p);
      t?.streams.add(ch);
      return ok(!!t);
    }),
    terminalRelease: vi.fn(async (ids: number[]) => {
      for (const t of server.terminals.values()) {
        for (const ch of t.streams) if (ids.includes(ch.id)) t.streams.delete(ch);
      }
      return ok(null);
    }),
    terminalClose: vi.fn(async () => ok(null)),
    listPtyAgents: vi.fn(async () =>
      ok([...server.terminals].filter(([, t]) => t.hooked).map(([id]) => agent(id, "idle"))),
    ),
  };
  const events = {
    ptyAgentChanged: {
      listen: (fn: (e: { payload: unknown }) => void) => {
        server.listeners.push(fn);
        return Promise.resolve(() => {});
      },
    },
  };
  const emit = (terminalId: string, state: string) =>
    server.listeners.forEach((fn) => fn({ payload: agent(terminalId, state) }));
  const output = (terminalId: string, frames: number, data: string) => {
    const t = server.terminals.get(terminalId);
    for (let i = 0; i < frames && t; i += 1) {
      const last = t.printed[t.printed.length - 1];
      const p = { offset: last ? last.offset + atob(last.data).length : 0, data };
      t.printed.push(p);
      for (const ch of t.streams) send(ch, p);
    }
  };
  // As the server ends a PTY: its open streams hear `exited`, and a hooked one's state changes.
  const exit = (terminalId: string) => {
    const t = server.terminals.get(terminalId);
    for (const ch of t?.streams ?? []) ch.onmessage({ kind: "exited", code: 0 });
    if (t?.hooked) emit(terminalId, "exited");
    server.terminals.delete(terminalId);
  };
  return { FakeChannel, server, commands, events, emit, output, exit };
});

vi.mock("@tauri-apps/api/core", () => ({ Channel: h.FakeChannel, invoke: vi.fn() }));
vi.mock("@/lib/bindings", () => ({ commands: h.commands, events: h.events }));
vi.mock("@/lib/log", () => ({ log: { warn: () => {}, error: () => {}, info: () => {}, debug: () => {} } }));
vi.mock("@/lib/notifications", () => ({ notifyNeedsAttention: async () => {} }));
vi.mock("@/lib/queryClient", () => ({
  queryClient: { setQueryData: () => {}, invalidateQueries: async () => {} },
}));
vi.mock("@/features/session/tabTitles", () => ({ adoptPtyTitle: () => {} }));
vi.mock("@/features/session/workstreamTabs", () => ({
  bindPtyTabTerminal: () => undefined,
  openAgentTerminalTab: () => {},
  setPtyTabSessionId: () => {},
  setPtyTabStarted: () => {},
  useWorkstreamTabsStore: { getState: () => ({ byWs: {} }) },
}));

const { ensureAgent, onAgentTerminalBack, onAgentTerminalGone, reconcileAgentTerminals } = await import(
  "./agentRuntime"
);
const { ptyAgentBridge } = await import("./ptyAgentBridge");
const { agentKey, useMultiplexerStore } = await import("./multiplexerStore");

const FRAME = btoa("x".repeat(48 * 1024));
const settle = () => new Promise((r) => setTimeout(r, 0));

let nextWorkstream = 0;
function opts(hooked: boolean, persistedTerminalId?: string) {
  h.server.nextHooked = hooked;
  return {
    workstreamId: `ws${++nextWorkstream}`,
    shellId: 1,
    engine: { id: "claude-code", label: "Claude" } as never,
    projectPath: "/p",
    workstreamTitle: "t",
    notifyOnBlocked: false,
    persistedTerminalId,
    session: { resume: false, bypass: false },
  };
}

async function launch(o: ReturnType<typeof opts>): Promise<string> {
  const launched = await ensureAgent(o);
  await settle();
  return launched!.terminalId;
}

function sentDuring(fn: () => void): number {
  const before = h.server.sent;
  fn();
  return h.server.sent - before;
}

beforeAll(() => {
  ptyAgentBridge();
});

describe("a PTY agent's output stream", () => {
  it("is not held for a hooked agent once it launches", async () => {
    const id = await launch(opts(true));
    expect(sentDuring(() => h.output(id, 200, FRAME)), "base64 bytes streamed after launch").toBe(0);
  });

  it("is dropped by a reattached agent once the server reports it hooked", async () => {
    h.server.terminals.set("adopt", { workstreamId: "", hooked: true, streams: new Set(), printed: [] });
    const o = opts(true, "adopt");
    h.server.terminals.get("adopt")!.workstreamId = o.workstreamId;
    await launch(o);
    h.emit("adopt", "working");
    await settle();
    expect(sentDuring(() => h.output("adopt", 200, FRAME)), "base64 bytes streamed after the report").toBe(0);
  });

  it("drives an unhooked agent's state", async () => {
    const o = opts(false);
    const id = await launch(o);
    h.output(id, 1, btoa("Do you want to proceed?"));
    await vi.waitFor(() =>
      expect(useMultiplexerStore.getState().agents[agentKey(o.workstreamId, 1)]?.state).toBe("blocked"),
    );
  });
});

describe("a hooked agent's exit", () => {
  it("reaches its runtime, so the tab launches anew instead of binding a dead terminal", async () => {
    const o = opts(true);
    const first = await launch(o);
    h.exit(first);
    await settle();
    const again = await ensureAgent(o);
    expect(again?.terminalId).toBeDefined();
    expect(again?.terminalId).not.toBe(first);
  });

  it("is not assumed when its session ends and restarts in the same process", async () => {
    const o = opts(true);
    const first = await launch(o);
    h.emit(first, "exited");
    h.emit(first, "idle");
    const launches = h.server.launches;
    const again = await ensureAgent(o);
    expect(again?.terminalId).toBe(first);
    expect(h.server.launches).toBe(launches);
  });
});

describe("a reconnect", () => {
  it("finds a gone hooked agent without reattaching any hooked stream", async () => {
    const live = await launch(opts(true));
    const goneOpts = opts(true);
    const gone = await launch(goneOpts);
    const unhooked = await launch(opts(false));
    h.server.terminals.delete(gone);
    const heard = vi.fn();
    const unsub = onAgentTerminalGone(goneOpts.workstreamId, 1, heard);
    h.server.attaches.length = 0;

    await reconcileAgentTerminals();
    unsub();

    expect(heard).toHaveBeenCalledWith(gone);
    const mine = [live, gone, unhooked];
    expect(h.server.attaches.filter((id) => mine.includes(id))).toEqual([unhooked]);
    expect(sentDuring(() => h.output(live, 1, FRAME))).toBe(0);
  });

  it("reattaches a live unhooked agent from where its output stopped", async () => {
    const id = await launch(opts(false));
    h.output(id, 3, FRAME);
    const before = h.server.sent;

    await reconcileAgentTerminals();

    expect(h.server.sent - before, "base64 bytes replayed").toBe(0);
    expect(sentDuring(() => h.output(id, 1, FRAME)), "the new stream carries new output").toBe(FRAME.length);
  });
});

describe("a server restart", () => {
  it("brings a gone hooked agent back to its tab when the server resumes it under the same id", async () => {
    const o = opts(true);
    const id = await launch(o);
    h.server.terminals.delete(id);
    const gone = vi.fn();
    const back = vi.fn();
    const unsubGone = onAgentTerminalGone(o.workstreamId, 1, gone);
    const unsubBack = onAgentTerminalBack(o.workstreamId, 1, back);

    await reconcileAgentTerminals();
    expect(gone).toHaveBeenCalledWith(id);
    h.server.terminals.set(id, { workstreamId: o.workstreamId, hooked: true, streams: new Set(), printed: [] });
    h.emit(id, "starting");
    h.emit(id, "working");
    unsubGone();
    unsubBack();

    expect(back).toHaveBeenCalledTimes(1);
    expect(back).toHaveBeenCalledWith(id);
    const launches = h.server.launches;
    expect((await ensureAgent(o))?.terminalId).toBe(id);
    expect(h.server.launches).toBe(launches);
  });

  it("never calls a live agent back", async () => {
    const o = opts(true);
    const id = await launch(o);
    const back = vi.fn();
    const unsub = onAgentTerminalBack(o.workstreamId, 1, back);
    h.emit(id, "working");
    unsub();
    expect(back).not.toHaveBeenCalled();
  });
});
