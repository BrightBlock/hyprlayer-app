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
  const interruptSession = vi.fn(async () => ({ status: "ok", data: null }));
  const killSession = vi.fn(async () => ({ status: "ok", data: null }));
  return {
    FakeChannel,
    state,
    startStageSession,
    sendUserMessage,
    resumeSession,
    interruptSession,
    killSession,
  };
});

vi.mock("@tauri-apps/api/core", () => ({ Channel: h.FakeChannel }));
vi.mock("../../lib/bindings", () => ({
  commands: {
    startStageSession: h.startStageSession,
    sendUserMessage: h.sendUserMessage,
    resumeSession: h.resumeSession,
    interruptSession: h.interruptSession,
    killSession: h.killSession,
  },
}));

// Imported after the mocks so the store binds to them.
const {
  startSession,
  sendMessage,
  cancelQueuedMessage,
  stopSession,
  classifyEntry,
  useSessionStore,
} = await import("./sessionStore");

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
  h.interruptSession.mockClear();
  h.killSession.mockClear();
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

describe("stopSession (interrupt → kill escalation)", () => {
  it("interrupts only when claude responds before the watchdog fires", async () => {
    vi.useFakeTimers();
    try {
      const id = await startBusySession();
      emit(INIT); // streaming → isWorking true → watchdog will arm

      const done = stopSession(id);
      // The graceful interrupt is sent immediately…
      expect(h.interruptSession).toHaveBeenCalledWith(id);
      // …then claude acks by emitting a turnDone, un-busying the session.
      emit(TURN_DONE);
      await vi.runAllTimersAsync();
      await done;

      // No escalation: the watchdog observed isWorking flip false in time.
      expect(h.killSession).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });

  it("escalates to killSession when claude doesn't ack within the grace window", async () => {
    vi.useFakeTimers();
    try {
      const id = await startBusySession();
      emit(INIT); // streaming → isWorking true

      const done = stopSession(id);
      expect(h.interruptSession).toHaveBeenCalledWith(id);
      // Claude never emits a result: drive past the grace window so the watchdog
      // gives up and the hard kill fires.
      await vi.runAllTimersAsync();
      await done;

      expect(h.killSession).toHaveBeenCalledWith(id);
    } finally {
      vi.useRealTimers();
    }
  });

  it("still kills when the interrupt itself fails to deliver", async () => {
    vi.useFakeTimers();
    try {
      // A wedged session whose stdin is gone — interrupt rejects, but we still
      // want the kill to land so the user isn't stuck.
      h.interruptSession.mockRejectedValueOnce(new Error("stdin closed"));
      const id = await startBusySession();
      emit(INIT);

      const done = stopSession(id);
      await vi.runAllTimersAsync();
      await done;

      expect(h.killSession).toHaveBeenCalledWith(id);
    } finally {
      vi.useRealTimers();
    }
  });
});

// ─── background task timestamps (taskStartedAt) ──────────────────────────────

const SPAWN_RESULT = (taskId: string) =>
  `Command running in background with ID: ${taskId}. Output is being written to: /tmp/tasks/${taskId}.output. You will be notified when it completes. To check interim output, use Read on that file path.`;

const BASH_BG_SPAWN = (toolUseId: string, _taskId: string): SessionEvent => ({
  kind: "toolUse",
  id: toolUseId,
  name: "Bash",
  input: JSON.stringify({ command: "sleep 999", run_in_background: true }),
  parentId: null,
});

const BASH_BG_RESULT = (toolUseId: string, taskId: string): SessionEvent => ({
  kind: "toolResult",
  toolUseId,
  content: SPAWN_RESULT(taskId),
  isError: false,
  parentId: null,
});

describe("taskStartedAt — per-task elapsed timestamps", () => {
  it("starts at {} when a session is created", async () => {
    const id = await startBusySession();
    expect(useSessionStore.getState().sessions[id]?.taskStartedAt).toEqual({});
  });

  it("stamps taskStartedAt when a background spawn result arrives", async () => {
    vi.useFakeTimers();
    try {
      const id = await startBusySession();
      const before = Date.now();
      emit(INIT);
      emit(BASH_BG_SPAWN("tu-1", "task-a"));
      emit(BASH_BG_RESULT("tu-1", "task-a"));

      const startedAt = useSessionStore.getState().sessions[id]?.taskStartedAt;
      expect(startedAt).toBeDefined();
      expect(startedAt?.["tu-1"]).toBeGreaterThanOrEqual(before);
    } finally {
      vi.useRealTimers();
    }
  });

  it("preserves the timestamp across subsequent events (monotonic)", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(BASH_BG_SPAWN("tu-1", "task-a"));
    emit(BASH_BG_RESULT("tu-1", "task-a"));

    const firstStamp = useSessionStore.getState().sessions[id]?.taskStartedAt?.["tu-1"];
    expect(firstStamp).toBeTruthy();

    // A new event arrives — stamp should remain unchanged.
    emit(TURN_DONE);
    const afterTurnDone = useSessionStore.getState().sessions[id]?.taskStartedAt?.["tu-1"];
    expect(afterTurnDone).toBe(firstStamp);
  });

  it("removes the timestamp when the task ends via TaskStop", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(BASH_BG_SPAWN("tu-1", "task-a"));
    emit(BASH_BG_RESULT("tu-1", "task-a"));
    emit(TURN_DONE);

    // Agent stops the task.
    const stopResult = JSON.stringify({
      message: "Successfully stopped task: task-a (sleep 999)",
      task_id: "task-a",
      task_type: "local_bash",
      command: "sleep 999",
    });
    emit({ kind: "toolUse", id: "ts-1", name: "TaskStop", input: JSON.stringify({ task_id: "task-a" }), parentId: null });
    emit({ kind: "toolResult", toolUseId: "ts-1", content: stopResult, isError: false, parentId: null });

    const taskStartedAt = useSessionStore.getState().sessions[id]?.taskStartedAt;
    expect(taskStartedAt?.["tu-1"]).toBeUndefined();
    expect(Object.keys(taskStartedAt ?? {})).toHaveLength(0);
  });
});

// ─── classifyEntry — background activity ────────────────────────────────────

const entryOf = (id: string) => useSessionStore.getState().sessions[id]!;

describe("classifyEntry — background activity state", () => {
  it("returns 'background' for a live resting session with a running background task", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(BASH_BG_SPAWN("tu-1", "task-a"));
    emit(BASH_BG_RESULT("tu-1", "task-a"));
    emit(TURN_DONE); // running && !streaming, task still un-stopped

    const entry = entryOf(id);
    expect(entry.state).toBe("running");
    expect(entry.streaming).toBe(false);
    expect(classifyEntry(entry)).toBe("background");
  });

  it("returns 'working' (which outranks background) while the turn is streaming", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(BASH_BG_SPAWN("tu-1", "task-a"));
    emit(BASH_BG_RESULT("tu-1", "task-a"));
    // No turnDone → still streaming → working takes precedence over background.
    const entry = entryOf(id);
    expect(entry.streaming).toBe(true);
    expect(classifyEntry(entry)).toBe("working");
  });

  it("returns 'review' (not 'background') for a COMPLETED session even with an un-stopped task", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(BASH_BG_SPAWN("tu-1", "task-a"));
    emit(BASH_BG_RESULT("tu-1", "task-a"));
    // The process exits cleanly while the shell is still un-stopped: it died
    // with the process group, so the dead session reads as review — not a
    // phantom "background" pulse.
    emit({ kind: "exited", outcome: "completed", code: 0, stderrTail: "" });

    const entry = entryOf(id);
    expect(entry.state).toBe("completed");
    expect(classifyEntry(entry)).toBe("review");
  });

  it("returns 'idle' for a KILLED session with an un-stopped task", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(BASH_BG_SPAWN("tu-1", "task-a"));
    emit(BASH_BG_RESULT("tu-1", "task-a"));
    emit({ kind: "exited", outcome: "killed", code: null, stderrTail: "" });

    const entry = entryOf(id);
    expect(entry.state).toBe("killed");
    expect(classifyEntry(entry)).toBe("idle");
  });
});
