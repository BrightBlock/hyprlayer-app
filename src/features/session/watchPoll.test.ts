// @vitest-environment jsdom
import { focusManager } from "@tanstack/react-query";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

// Its own file: the watch poll installs once per module and never uninstalls.
const h = vi.hoisted(() => {
  const listeners = new Map<string, (event: { payload: unknown }) => void>();
  const listen = (name: string) => ({
    listen: async (cb: (event: { payload: unknown }) => void) => {
      listeners.set(name, cb);
      return () => {};
    },
  });
  return {
    listeners,
    events: new Proxy({}, { get: (_, name) => listen(String(name)) }),
    watchSessions: vi.fn(),
    working: false,
    nextId: 0,
  };
});

vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    onmessage: ((item: unknown) => void) | null = null;
  },
}));
vi.mock("../../lib/bindings", () => ({
  commands: {
    connectionStatus: async () => ({ status: "ok", data: { state: "connected", transcriptSync: true } }),
    getClientIdentity: async () => ({ status: "ok", data: { clientId: "me", device: "my-mac" } }),
    listActiveSessions: async () => ({ status: "ok", data: [] }),
    startStageSession: async () => ({
      status: "ok",
      data: { session: { id: `s${(h.nextId += 1)}`, model: "m", claudeSessionId: "c" }, prompt: "kick off" },
    }),
    watchSessions: h.watchSessions,
  },
  events: h.events,
}));

const { dropSession, startSession } = await import("./sessionStore");
const { scheduleWatch, setupTranscriptSync, transcriptSyncBridge, transcriptSyncReconnected } =
  await import("./syncStore");
const { attachEventListeners, onEventsMissed } = await import("@/lib/events");

h.watchSessions.mockImplementation(async (sessions: { sessionId: string; rev: number }[]) => ({
  status: "ok",
  data: sessions.map(({ sessionId, rev }) => ({
    sessionId,
    rev,
    resetRev: 0,
    working: h.working,
    inFlight: 0,
    viewers: [],
  })),
}));

const MINUTE = 60_000;
const calls = () => h.watchSessions.mock.calls.length;
const watchedIn = (call: number) => (h.watchSessions.mock.calls[call]![0] as { sessionId: string }[]).map((s) => s.sessionId);
/** Seconds until the next watch; called right after one, the gap between two. */
async function nextGap(limit = 5 * MINUTE): Promise<number> {
  const before = calls();
  for (let waited = 0; waited < limit; waited += 1_000) {
    await vi.advanceTimersByTimeAsync(1_000);
    if (calls() > before) return waited + 1_000;
  }
  return Infinity;
}
async function open(): Promise<string> {
  const { id } = await startSession({ workstreamId: "ws1", stage: "general", input: "kick off" });
  return id;
}

beforeAll(async () => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval"] });
  // As main.tsx wires them.
  onEventsMissed(scheduleWatch);
  attachEventListeners();
  for (const off of transcriptSyncBridge()) await off;
  setupTranscriptSync();
  await vi.advanceTimersByTimeAsync(1_000);
});

beforeEach(() => {
  h.working = false;
});

describe("the watch poll", () => {
  it("sends nothing in ten idle minutes with no session open", async () => {
    h.watchSessions.mockClear();
    await vi.advanceTimersByTimeAsync(10 * MINUTE);

    expect(calls()).toBe(0);
  });

  it("tells the server once when the last open session closes, then goes quiet", async () => {
    const id = await open();
    await vi.advanceTimersByTimeAsync(1_000);
    h.watchSessions.mockClear();

    dropSession(id);
    await vi.advanceTimersByTimeAsync(10 * MINUTE);

    expect(calls()).toBe(1);
    expect(watchedIn(0)).toEqual([]);
  });

  it("backs off to once a minute while every open session is idle", async () => {
    const id = await open();
    await vi.advanceTimersByTimeAsync(1_000);
    h.watchSessions.mockClear();

    await vi.advanceTimersByTimeAsync(10 * MINUTE);

    expect(calls()).toBeLessThanOrEqual(11);
    await nextGap();
    expect(await nextGap()).toBe(MINUTE);
    dropSession(id);
  });

  it("keeps polling every 15 s while a session is working", async () => {
    const id = await open();
    h.working = true;
    await vi.advanceTimersByTimeAsync(5 * MINUTE);
    await nextGap();

    expect(await nextGap()).toBe(15_000);
    expect(await nextGap()).toBe(15_000);
    dropSession(id);
  });

  it("watches at once on EventsMissed, a reconnect and focus, then polls quickly again", async () => {
    const id = await open();
    await vi.advanceTimersByTimeAsync(10 * MINUTE);
    const soon = async (trigger: () => void) => {
      const before = calls();
      trigger();
      await vi.advanceTimersByTimeAsync(1_000);
      return calls() > before;
    };

    expect(await soon(() => h.listeners.get("eventsMissed")!({ payload: null }))).toBe(true);
    expect(await soon(transcriptSyncReconnected)).toBe(true);
    focusManager.setFocused(false);
    expect(await soon(() => focusManager.setFocused(true))).toBe(true);
    expect(await nextGap()).toBeLessThanOrEqual(30_000);
    dropSession(id);
  });
});
