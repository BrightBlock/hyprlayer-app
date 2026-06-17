import { beforeEach, describe, expect, it, vi } from "vitest";

import type { SessionEvent } from "../../lib/bindings";

// Hoisted so the (hoisted) vi.mock factories below can reference these safely.
// A `FakeChannel` stands in for the Tauri `Channel`: `onmessage` is just a
// settable callback the store wires up, and the tests drive events by calling
// it. `startStageSession` captures the channel it's handed (the store's event
// sink) and hands back a minimal Session row.
const h = vi.hoisted(() => {
  class FakeChannel {
    onmessage: ((event: unknown) => void) | null = null;
  }
  const state = { lastChannel: null as FakeChannel | null, nextId: 0 };
  const startStageSession = vi.fn(async (...args: unknown[]) => {
    state.lastChannel = args[args.length - 1] as FakeChannel;
    const id = `s${(state.nextId += 1)}`;
    return { status: "ok", data: { id, model: "claude-opus-4-8", claudeSessionId: `c-${id}` } };
  });
  const sendUserMessage = vi.fn(async () => ({ status: "ok", data: null }));
  const resumeSession = vi.fn(async () => ({ status: "ok", data: {} }));
  return { FakeChannel, state, startStageSession, sendUserMessage, resumeSession };
});

vi.mock("@tauri-apps/api/core", () => ({ Channel: h.FakeChannel }));
vi.mock("../../lib/bindings", () => ({
  commands: {
    startStageSession: h.startStageSession,
    sendUserMessage: h.sendUserMessage,
    resumeSession: h.resumeSession,
  },
}));

// Imported after the mocks so the store binds to them.
const { startSession, sendMessage, cancelQueuedMessage, useSessionStore } = await import(
  "./sessionStore"
);

const INIT: SessionEvent = { kind: "init", claudeSessionId: "c1", model: "m" };
const TURN_DONE: SessionEvent = {
  kind: "turnDone",
  costUsd: null,
  numTurns: null,
  contextTokens: null,
};

/** Drive an event through the live channel, exactly like the backend would. */
function emit(event: SessionEvent): void {
  h.state.lastChannel?.onmessage?.(event);
}

async function startBusySession(): Promise<string> {
  const session = await startSession({
    workstreamId: "ws1",
    stage: "general",
    input: "kick off",
  });
  return session.id;
}

function queued(id: string): string[] {
  return (useSessionStore.getState().sessions[id]?.queued ?? []).map((m) => m.text);
}

beforeEach(() => {
  useSessionStore.setState({ sessions: {} });
  h.startStageSession.mockClear();
  h.sendUserMessage.mockClear();
  h.resumeSession.mockClear();
  h.state.lastChannel = null;
});

describe("conversation steering (mid-turn message queue)", () => {
  it("queues a message sent while a turn is in flight instead of sending it", async () => {
    const id = await startBusySession(); // state "starting" → busy
    await sendMessage(id, "follow-up");

    expect(queued(id)).toEqual(["follow-up"]);
    expect(h.sendUserMessage).not.toHaveBeenCalled();
  });

  it("delivers queued messages one per turn as turns finish, in order", async () => {
    const id = await startBusySession();
    await sendMessage(id, "A");
    await sendMessage(id, "B");
    expect(queued(id)).toEqual(["A", "B"]);

    emit(INIT);
    emit(TURN_DONE); // first turn done → flush "A"
    await Promise.resolve();
    expect(h.sendUserMessage).toHaveBeenLastCalledWith(id, "A", []);
    expect(queued(id)).toEqual(["B"]);

    emit(TURN_DONE); // "A"'s turn done → flush "B"
    await Promise.resolve();
    expect(h.sendUserMessage).toHaveBeenLastCalledWith(id, "B", []);
    expect(queued(id)).toEqual([]);
  });

  it("holds queued messages while the session is parked on a permission", async () => {
    const id = await startBusySession();
    await sendMessage(id, "later");

    // A turn that ends parked on a permission must NOT flush the queue.
    emit({ kind: "permissionAsk", requestId: "req-1", toolName: "Bash", input: "{}" });
    emit(TURN_DONE);
    await Promise.resolve();
    expect(h.sendUserMessage).not.toHaveBeenCalled();
    expect(queued(id)).toEqual(["later"]);
  });

  it("cancelQueuedMessage drops a message before it's delivered", async () => {
    const id = await startBusySession();
    await sendMessage(id, "keep");
    await sendMessage(id, "drop");

    cancelQueuedMessage(id, 1);
    expect(queued(id)).toEqual(["keep"]);
  });
});
