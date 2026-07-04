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
    // Mirrors the backend's StartedSession: the row plus the composed prompt
    // (here just the raw input — composition is the backend's concern).
    return {
      status: "ok",
      data: {
        session: { id, model: "claude-opus-4-8", claudeSessionId: `c-${id}` },
        prompt: String(args[2] ?? ""),
      },
    };
  });
  const sendUserMessage = vi.fn(async () => ({ status: "ok", data: null }));
  const resumeSession = vi.fn(async () => ({ status: "ok", data: {} }));
  const interruptSession = vi.fn(async () => ({ status: "ok", data: null }));
  const killSession = vi.fn(async () => ({ status: "ok", data: null }));
  const respondPermission = vi.fn(async () => ({ status: "ok", data: null }));
  const answerQuestion = vi.fn(async () => ({ status: "ok", data: null }));
  // A permission-mode restart: capture the fresh channel the store hands over so
  // post-restart `emit`s reach it, exactly like `startStageSession`.
  const restartSession = vi.fn(async (...args: unknown[]) => {
    state.lastChannel = args[args.length - 1] as FakeChannel;
    return { status: "ok", data: null };
  });
  return {
    FakeChannel,
    state,
    startStageSession,
    sendUserMessage,
    resumeSession,
    interruptSession,
    killSession,
    respondPermission,
    answerQuestion,
    restartSession,
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
    respondPermission: h.respondPermission,
    answerQuestion: h.answerQuestion,
    restartSession: h.restartSession,
  },
}));

// Imported after the mocks so the store binds to them.
const {
  startSession,
  sendMessage,
  cancelQueuedMessage,
  stopSession,
  resumeSession,
  applyModel,
  applyPermissionMode,
  classifyEntry,
  workstreamShipBusy,
  useSessionStore,
  usePlanDrawerStore,
  setPlanDrawerUi,
  togglePlanDrawerCollapsed,
} = await import("./sessionStore");
const { isBusy, isWorking } = await import("./transcript");

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
  usePlanDrawerStore.setState({ byId: {} });
  h.startStageSession.mockClear();
  h.sendUserMessage.mockClear();
  h.resumeSession.mockClear();
  h.interruptSession.mockClear();
  h.killSession.mockClear();
  h.respondPermission.mockClear();
  h.answerQuestion.mockClear();
  h.restartSession.mockClear();
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

  it("removes the timestamp when the task completes via notification (no poll)", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(BASH_BG_SPAWN("tu-1", "task-a"));
    emit(BASH_BG_RESULT("tu-1", "task-a"));
    emit(TURN_DONE);
    expect(useSessionStore.getState().sessions[id]?.taskStartedAt?.["tu-1"]).toBeTruthy();

    // claude's async completion notification — the agent never polled the task.
    emit({ kind: "backgroundTaskUpdate", taskId: "task-a", toolUseId: "tu-1", status: "completed" });

    const taskStartedAt = useSessionStore.getState().sessions[id]?.taskStartedAt;
    expect(taskStartedAt?.["tu-1"]).toBeUndefined();
    expect(Object.keys(taskStartedAt ?? {})).toHaveLength(0);
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

// ─── recovering a turn parked on a permission/question ───────────────────────
// Changing the permission mode or hitting Stop while claude is asking for an
// approval/question used to strand the session `isBusy` forever (the dangling
// card kept the composer locked and the queue frozen, with no live claude able
// to answer the dead request). These guard the recovery.

describe("parked-turn recovery (permission mode change / stop)", () => {
  const PERMISSION_ASK: SessionEvent = {
    kind: "permissionAsk",
    requestId: "req-1",
    toolName: "Bash",
    input: "{}",
  };
  const QUESTION_ASK: SessionEvent = {
    kind: "questionAsk",
    requestId: "q-1",
    questions: JSON.stringify([
      { question: "Which?", header: "Pick", multiSelect: false, options: [{ label: "A", description: "" }] },
    ]),
  };

  it("changing the permission mode while parked on a permission un-sticks the session", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(PERMISSION_ASK);
    expect(isBusy(entryOf(id))).toBe(true); // parked → composer locked

    await applyPermissionMode(id, "bypassPermissions"); // restart into the new mode
    await Promise.resolve();

    // A mode-only restart passes null model/effort (claude keeps the session's
    // originals on --resume); the new mode rides in the permission_mode slot.
    expect(h.restartSession).toHaveBeenCalledWith(id, null, null, "bypassPermissions", expect.anything());
    const entry = entryOf(id);
    expect(entry.pendingPermission).toBeNull();
    // The dangling card is resolved (deny = the tool never ran), so isBusy clears.
    expect(entry.items.some((it) => it.kind === "permission" && it.decision === null)).toBe(false);
    expect(isBusy(entry)).toBe(false);
  });

  it("auto-sends queued follow-ups after a permission-mode restart", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(PERMISSION_ASK);
    await sendMessage(id, "do it now"); // parked → queued, not sent
    expect(queued(id)).toEqual(["do it now"]);
    expect(h.sendUserMessage).not.toHaveBeenCalled();

    await applyPermissionMode(id, "acceptEdits");
    await Promise.resolve();

    // The restart freed the turn, so the queue flushes under the new mode.
    expect(h.sendUserMessage).toHaveBeenCalledWith(id, "do it now", []);
    expect(queued(id)).toEqual([]);
  });

  it("switching the model on a live session resumes it on the new model", async () => {
    const id = await startBusySession(); // live session (channel bound)
    emit(INIT);

    // applyModel(id, modelFlag, effort, mode) relaunches via --resume onto the
    // new model → commands.restartSession(id, model, effort, permission_mode, ch).
    await applyModel(id, "sonnet[1m]", "ultracode", "default");
    await Promise.resolve();

    expect(h.restartSession).toHaveBeenCalledWith(
      id,
      "sonnet[1m]",
      "ultracode",
      "default",
      expect.anything(),
    );
  });

  it("stop while parked on a permission declines the live ask, interrupts, and unlocks", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(PERMISSION_ASK);
    expect(isBusy(entryOf(id))).toBe(true);

    await stopSession(id);

    // The decline goes to the LIVE driver (releases a claude blocked on the ask),
    // then the interrupt halts the turn. No hard kill — it wasn't wedged-computing.
    expect(h.respondPermission).toHaveBeenCalledWith(id, "req-1", false, null);
    expect(h.interruptSession).toHaveBeenCalledWith(id);
    expect(h.killSession).not.toHaveBeenCalled();

    const entry = entryOf(id);
    expect(entry.pendingPermission).toBeNull();
    expect(isBusy(entry)).toBe(false);
  });

  it("stop while parked on a question dismisses it, interrupts, and unlocks", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit({ kind: "toolUse", id: "tu-q", name: "AskUserQuestion", input: "{}", parentId: null });
    emit(QUESTION_ASK);
    expect(entryOf(id).pendingQuestion).not.toBeNull();

    await stopSession(id);

    expect(h.answerQuestion).toHaveBeenCalledWith(id, "q-1", "{}"); // empty = dismiss
    expect(h.interruptSession).toHaveBeenCalledWith(id);
    expect(h.respondPermission).not.toHaveBeenCalled();

    const entry = entryOf(id);
    expect(entry.pendingQuestion).toBeNull();
    expect(isBusy(entry)).toBe(false);
  });
});

// ─── resuming a silent process ───────────────────────────────────────────────
// A `--resume`d claude emits `init` only with a turn, never on bare spawn, so a
// re-attach onto a quiet process leaves the session in `starting` forever:
// "Connecting…" never clears, the activity pill keeps pulsing (`isWorking` stays
// true), and a queued follow-up never flushes. resumeSession settles out of
// `starting` once the (empty) resume burst has flushed.

const EXITED: SessionEvent = { kind: "exited", outcome: "crashed", code: 1, stderrTail: "" };

describe("resuming a silent process settles out of `starting`", () => {
  it("a silent resume settles to a resting state instead of pulsing 'Connecting'", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(TURN_DONE);
    emit(EXITED); // process gone, channel dropped → detached

    await resumeSession(id); // backend re-attaches; the process stays silent (no events)

    const entry = entryOf(id);
    expect(entry.state).not.toBe("starting"); // not stuck "Connecting…"
    expect(isWorking(entry)).toBe(false); // pill stops pulsing
    expect(classifyEntry(entry)).not.toBe("working");
  });

  it("flushes a queued follow-up after a silent resume", async () => {
    const id = await startBusySession(); // "starting" → busy
    await sendMessage(id, "later"); // queued behind the in-flight turn
    expect(queued(id)).toEqual(["later"]);
    emit(EXITED); // turn dies with a follow-up still queued → detached

    await resumeSession(id);

    // The resumed process is idle, so the queued turn is released rather than
    // sitting forever behind a turn that will never arrive.
    expect(h.sendUserMessage).toHaveBeenCalledWith(id, "later", []);
    expect(queued(id)).toEqual([]);
  });

  it("does not clobber a resume that re-attaches onto a genuinely mid-turn process", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(TURN_DONE);
    emit(EXITED); // detach

    // Resume, but this time the re-attached process IS mid-turn: its stream lands
    // during the resume burst, advancing us past `starting`. The settle must leave
    // that real working state alone.
    h.resumeSession.mockImplementationOnce(async (...args: unknown[]) => {
      const ch = args[args.length - 1] as InstanceType<typeof h.FakeChannel>;
      ch.onmessage?.({ kind: "partialText", text: "thinking" } as SessionEvent);
      return { status: "ok", data: {} };
    });

    await resumeSession(id);

    expect(isWorking(entryOf(id))).toBe(true); // still streaming — not settled to idle
  });
});

// ─── closing the streaming caret on turn abandonment ─────────────────────────
// An open assistant block renders a blinking caret while `streaming: true`. A
// turn that's abandoned (permission-mode restart, or a mid-turn crash) used to
// settle the session's `streaming` flag but leave the block's own flag set, so
// the caret kept pulsing after the conversation had already stopped.

const PARTIAL = (text: string): SessionEvent =>
  ({ kind: "partialText", text, parentId: null }) as SessionEvent;
const hasStreamingBlock = (id: string) =>
  entryOf(id).items.some((it) => it.kind === "assistant" && it.streaming);

describe("turn abandonment closes the streaming caret", () => {
  it("a permission-mode restart mid-stream closes the open assistant block", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(PARTIAL("thinking…"));
    expect(hasStreamingBlock(id)).toBe(true); // caret pulsing

    await applyPermissionMode(id, "bypassPermissions"); // restart interrupts the turn
    await Promise.resolve();

    expect(hasStreamingBlock(id)).toBe(false); // caret stopped
    expect(entryOf(id).streaming).toBe(false);
  });

  it("a crash mid-stream closes the open assistant block", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(PARTIAL("thinking…"));
    expect(hasStreamingBlock(id)).toBe(true);

    emit(EXITED); // process dies mid-turn

    expect(hasStreamingBlock(id)).toBe(false);
  });
});

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

  it("returns 'review' (not 'background') for a DETACHED session (live:false)", async () => {
    // Restored-after-restart: no live process, so its leftover spawn isn't running.
    const id = await startBusySession();
    emit(INIT);
    emit(BASH_BG_SPAWN("tu-1", "task-a"));
    emit(BASH_BG_RESULT("tu-1", "task-a"));
    emit(TURN_DONE);

    const live = entryOf(id);
    expect(classifyEntry(live)).toBe("background"); // sanity: live → background
    expect(classifyEntry({ ...live, live: false })).toBe("review");
  });

  it("flips from 'background' to 'review' once the task completes via notification", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(BASH_BG_SPAWN("tu-1", "task-a"));
    emit(BASH_BG_RESULT("tu-1", "task-a"));
    emit(TURN_DONE);
    expect(classifyEntry(entryOf(id))).toBe("background");

    // The screenshot bug: the task finished but stayed "background" forever
    // because the agent never polled it. The async notification now clears it.
    emit({ kind: "backgroundTaskUpdate", taskId: "task-a", toolUseId: "tu-1", status: "completed" });

    const entry = entryOf(id);
    expect(entry.state).toBe("running");
    expect(entry.streaming).toBe(false);
    expect(classifyEntry(entry)).toBe("review");
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

// ─── workstreamShipBusy — hide Commit while Ship commits ────────────────────

describe("workstreamShipBusy", () => {
  const shipBusy = (ws: string) => workstreamShipBusy(useSessionStore.getState().sessions, ws);

  it("is true while the ship session is mid-turn, false once that turn parks", async () => {
    await startSession({ workstreamId: "ws1", stage: "ship", input: "" });
    expect(shipBusy("ws1")).toBe(true); // "starting" → working

    emit(INIT); // streaming
    expect(shipBusy("ws1")).toBe(true);

    emit(TURN_DONE); // commit/push/PR done, parks at "review"
    expect(shipBusy("ws1")).toBe(false);
  });

  it("stays true while the ship turn is paused on a permission (needs-you)", async () => {
    await startSession({ workstreamId: "ws1", stage: "ship", input: "" });
    emit(INIT);
    emit({ kind: "permissionAsk", requestId: "req-1", toolName: "Bash", input: "{}" });
    expect(shipBusy("ws1")).toBe(true);
  });

  it("ignores non-ship sessions and other workstreams' ship sessions", async () => {
    // A working *general*-stage session must not suppress Commit…
    await startSession({ workstreamId: "ws1", stage: "general", input: "" });
    expect(shipBusy("ws1")).toBe(false);

    // …nor a ship session belonging to a different workstream.
    await startSession({ workstreamId: "ws2", stage: "ship", input: "" });
    expect(shipBusy("ws1")).toBe(false);
    expect(shipBusy("ws2")).toBe(true);
  });
});

describe("plan drawer view state (per-session, survives tab switch)", () => {
  const drawer = (id: string) => usePlanDrawerStore.getState().byId[id];

  it("patches over the collapsed/not-dismissed default", () => {
    setPlanDrawerUi("s1", { dismissed: true });
    expect(drawer("s1")).toEqual({ collapsed: true, dismissed: true });
  });

  it("keeps each session's state isolated — the tab-switch bug", () => {
    // Finish + dismiss s1's plan, then move to a fresh tab (s2) and back.
    setPlanDrawerUi("s1", { dismissed: true, collapsed: true });
    setPlanDrawerUi("s2", { dismissed: false, collapsed: false });
    // s1 stays dismissed: leaving and returning to its tab must not revive it.
    expect(drawer("s1")).toEqual({ collapsed: true, dismissed: true });
    expect(drawer("s2")).toEqual({ collapsed: false, dismissed: false });
  });

  it("toggles only the collapsed flag", () => {
    setPlanDrawerUi("s1", { dismissed: true });
    togglePlanDrawerCollapsed("s1"); // true -> false
    expect(drawer("s1")).toEqual({ collapsed: false, dismissed: true });
    togglePlanDrawerCollapsed("s1"); // false -> true
    expect(drawer("s1")).toEqual({ collapsed: true, dismissed: true });
  });

  it("is a no-op when nothing changes (so the auto-clear effect can re-fire freely)", () => {
    setPlanDrawerUi("s1", { dismissed: true, collapsed: true });
    const before = drawer("s1");
    setPlanDrawerUi("s1", { dismissed: true, collapsed: true });
    expect(drawer("s1")).toBe(before); // same reference: no churn
  });
});
