// @vitest-environment jsdom
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import type { LogCursorDto, SessionEvent } from "@/lib/bindings";

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
    listSessionTurns: vi.fn(),
    watchSessions: vi.fn(),
    working: new Set<string>(),
    channel: null as { onmessage: ((item: unknown) => void) | null } | null,
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
    startStageSession: async (...args: unknown[]) => {
      h.channel = args[args.length - 1] as typeof h.channel;
      return { status: "ok", data: { session: { id: "started", model: "m", claudeSessionId: "c" }, prompt: "kick off" } };
    },
    listSessionTurns: h.listSessionTurns,
    watchSessions: h.watchSessions,
  },
  events: h.events,
}));

const { restoreSessionIfNeeded, startSession, useSessionStore } = await import("./sessionStore");
const { scheduleWatch, setupTranscriptSync, syncOf, transcriptSyncBridge, useTranscriptSync } =
  await import("./syncStore");
const { attachEventListeners, onEventsMissed } = await import("@/lib/events");

type Row = { seq: number; kind: string; event: SessionEvent; createdAt: number };
const REV_KINDS = new Set(["userMessage", "assistantText", "toolResult", "exited"]);
const SELF = { clientId: "me", device: "my-mac" };
const OTHER = { clientId: "them", device: "my-phone" };

const crashed: SessionEvent = { kind: "exited", outcome: "crashed", code: 1, stderrTail: "boom" };
const refused: SessionEvent = { kind: "exited", outcome: "spawn_failed", code: null, stderrTail: "no such model" };
const ENDED: SessionEvent[] = [
  { kind: "init", claudeSessionId: "c1", model: "m" },
  { kind: "userMessage", text: "kick off", stage: null, author: SELF },
  { kind: "assistantText", text: "done", parentId: null },
  { kind: "turnDone", costUsd: null, numTurns: null, contextTokens: null },
  crashed,
];

// The server as it is: rows past the cursor it is handed, and the rev they add up to.
const server = new Map<string, Row[]>();
const append = (id: string, event: SessionEvent) => {
  const rows = server.get(id)!;
  rows.push({ seq: rows.length, kind: event.kind, event, createdAt: 1_700_000_000_000 + rows.length });
};
const revOf = (id: string) => server.get(id)!.filter((r) => REV_KINDS.has(r.kind)).length;
const cursorAt = (row: Row): LogCursorDto => ({ kind: "after", seq: row.seq, createdAt: row.createdAt });

h.listSessionTurns.mockImplementation(async (id: string, cursor: LogCursorDto, limit: number) => {
  const rows = server.get(id)!;
  const from = cursor.kind === "after" ? (cursor.seq ?? -1) + 1 : 0;
  const entries = rows.slice(from, from + limit);
  const last = entries[entries.length - 1];
  return {
    status: "ok",
    data: {
      entries,
      nextCursor: last ? cursorAt(last) : cursor,
      remaining: Math.max(0, rows.length - from - entries.length),
      rev: revOf(id),
      resetRev: 0,
      working: false,
      inFlight: 0,
    },
  };
});
h.watchSessions.mockImplementation(async (sessions: { sessionId: string }[]) => ({
  status: "ok",
  data: sessions.map(({ sessionId }) => ({
    sessionId,
    rev: revOf(sessionId),
    resetRev: 0,
    working: h.working.has(sessionId),
    inFlight: 0,
    viewers: [],
  })),
}));

const entryOf = (id: string) => useSessionStore.getState().sessions[id]!;
const reads = () => (h.listSessionTurns.mock.calls as unknown[][]).map((c) => [c[0], c[1]]);
const kinds = (id: string) => entryOf(id).items.map((it) => it.kind);

let nextId = 0;
async function openEnded(): Promise<string> {
  const id = `ended-${(nextId += 1)}`;
  server.set(id, []);
  for (const event of ENDED) append(id, event);
  await restoreSessionIfNeeded(id, "ws1", "general");
  expect(entryOf(id).state).toBe("crashed");
  h.listSessionTurns.mockClear();
  return id;
}

const poll = () => vi.advanceTimersByTimeAsync(15_000);

beforeAll(async () => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval"] });
  // As main.tsx wires them.
  onEventsMissed(scheduleWatch);
  attachEventListeners();
  for (const off of transcriptSyncBridge()) await off;
  setupTranscriptSync();
  await vi.advanceTimersByTimeAsync(300);
});

beforeEach(() => {
  h.working.clear();
  useSessionStore.setState({ sessions: {} });
  useTranscriptSync.setState({ bySession: {}, self: SELF });
  h.watchSessions.mockClear();
  h.listSessionTurns.mockClear();
});

afterEach(() => {
  useSessionStore.setState({ sessions: {} });
});

describe("an ended view that missed its revision", () => {
  it("catches up on the next watch poll, reading only past its cursor", async () => {
    const id = await openEnded();
    const tail = server.get(id)![4];
    // Another device's refused retry logs an exit; this view never hears of it.
    append(id, refused);

    await poll();

    expect(h.watchSessions).toHaveBeenCalled();
    await vi.waitFor(() => expect(kinds(id).filter((k) => k === "exit")).toHaveLength(2));
    expect(reads()).toEqual([[id, cursorAt(tail)]]);
    expect(kinds(id).filter((k) => k === "user")).toHaveLength(1);
    expect(kinds(id).filter((k) => k === "assistant")).toHaveLength(1);
    expect(syncOf(id)).toMatchObject({ localRev: revOf(id), phase: "inSync" });

    // Caught up: the next poll reads nothing.
    await poll();
    expect(reads()).toHaveLength(1);
    expect(kinds(id).filter((k) => k === "exit")).toHaveLength(2);
  });

  it("catches up on an EventsMissed resync", async () => {
    const id = await openEnded();
    const tail = server.get(id)![4];
    append(id, refused);

    h.listeners.get("eventsMissed")!({ payload: null });
    await vi.advanceTimersByTimeAsync(300);

    expect(h.watchSessions).toHaveBeenCalled();
    await vi.waitFor(() => expect(kinds(id).filter((k) => k === "exit")).toHaveLength(2));
    expect(reads()).toEqual([[id, cursorAt(tail)]]);
    expect(kinds(id).filter((k) => k === "user")).toHaveLength(1);
    expect(syncOf(id)).toMatchObject({ localRev: revOf(id), phase: "inSync" });
  });

  it("loads another device's turn it was told of once that turn ends", async () => {
    const id = await openEnded();
    const tail = server.get(id)![4];
    const revised = h.listeners.get("sessionRevised")!;
    const told = (turns: { seq: number; kind: string }[], working: boolean) =>
      revised({
        payload: {
          sessionId: id,
          workstreamId: "ws1",
          rev: revOf(id),
          turns,
          author: working ? OTHER : null,
          working,
          at: 0,
        },
      });
    // The phone resumes the session and runs a turn.
    h.working.add(id);
    append(id, { kind: "init", claudeSessionId: "c1", model: "m" });
    append(id, { kind: "userMessage", text: "from the phone", stage: null, author: OTHER });
    told([{ seq: 6, kind: "user" }], true);
    append(id, { kind: "assistantText", text: "theirs", parentId: null });
    told([{ seq: 7, kind: "assistant" }], true);
    await poll();
    expect(reads()).toEqual([]);

    append(id, { kind: "turnDone", costUsd: null, numTurns: null, contextTokens: null });
    h.working.delete(id);
    told([], false);

    await vi.waitFor(() => expect(kinds(id).filter((k) => k === "user")).toHaveLength(2));
    expect(reads()).toEqual([[id, cursorAt(tail)]]);
    expect(kinds(id).filter((k) => k === "assistant")).toHaveLength(2);
    expect(syncOf(id)).toMatchObject({ localRev: revOf(id), phase: "inSync" });
    // Resumed elsewhere, so no longer ended: its composer sends through a resume of its own.
    expect(entryOf(id)).toMatchObject({ state: "running", streaming: false, live: false });
  });

  it("waits out another device's turn that is still running", async () => {
    const id = await openEnded();
    append(id, { kind: "init", claudeSessionId: "c1", model: "m" });
    append(id, { kind: "userMessage", text: "from the phone", stage: null, author: OTHER });
    h.working.add(id);

    await poll();

    expect(h.watchSessions).toHaveBeenCalled();
    expect(reads()).toEqual([]);
    expect(entryOf(id).state).toBe("crashed");
  });

  it("catches up a view that ended from its own stream, from its last streamed row", async () => {
    const { id } = await startSession({ workstreamId: "ws1", stage: "general", input: "kick off" });
    server.set(id, []);
    for (const event of ENDED.filter((e) => e.kind !== "userMessage")) {
      append(id, event);
      const rows = server.get(id)!;
      const row = rows[rows.length - 1];
      h.channel!.onmessage?.({ seq: row.seq, createdAt: row.createdAt, rev: revOf(id), author: SELF, event });
    }
    expect(entryOf(id).state).toBe("crashed");
    const tail = server.get(id)![server.get(id)!.length - 1];

    await poll();
    expect(reads()).toEqual([]);

    append(id, refused);
    // An idle view polls once a minute at most.
    await vi.advanceTimersByTimeAsync(60_000);

    await vi.waitFor(() => expect(kinds(id).filter((k) => k === "exit")).toHaveLength(2));
    expect(reads()).toEqual([[id, cursorAt(tail)]]);
    expect(kinds(id).filter((k) => k === "assistant")).toHaveLength(1);
  });
});
