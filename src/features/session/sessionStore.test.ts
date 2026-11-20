// @vitest-environment jsdom
import { cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { GoalBrief, LogCursorDto, SessionEvent } from "@/lib/bindings";

const h = vi.hoisted(() => {
  class FakeChannel {
    onmessage: ((event: unknown) => void) | null = null;
  }
  const state = { lastChannel: null as FakeChannel | null, nextId: 0 };
  const startStageSession = vi.fn(async (...args: unknown[]) => {
    state.lastChannel = args[args.length - 1] as FakeChannel;
    const id = `s${(state.nextId += 1)}`;
    return {
      status: "ok",
      data: {
        session: { id, model: "claude-opus-4-8", claudeSessionId: `c-${id}` },
        prompt: String(args[2] ?? ""),
      },
    };
  });
  const sendUserMessage = vi.fn(async () => ({ status: "ok", data: null }));
  // A ResumedSession whose harness is unknown, so a busy send queues for the turn's end.
  const RESUMED = { session: {}, permissionMode: null };
  const resumeSession = vi.fn(async () => ({ status: "ok", data: RESUMED }));
  const interruptSession = vi.fn(async () => ({ status: "ok", data: null }));
  const killSession = vi.fn(async () => ({ status: "ok", data: null }));
  const respondPermission = vi.fn(async () => ({ status: "ok", data: null }));
  const answerQuestion = vi.fn(async () => ({ status: "ok", data: null }));
  const getSessionLog = vi.fn(async () => ({ status: "ok", data: [] as unknown[] }));
  const listActiveSessions = vi.fn(async () => ({ status: "ok", data: [] as unknown[] }));
  const restartSession = vi.fn(async (...args: unknown[]) => {
    state.lastChannel = args[args.length - 1] as FakeChannel;
    return { status: "ok", data: null };
  });
  const connectionStatus = vi.fn(async () => ({
    status: "ok",
    data: { state: "connected", transcriptSync: false },
  }));
  const listSessionTurns = vi.fn();
  const getClientIdentity = vi.fn(async () => ({
    status: "ok",
    data: { clientId: "me", device: "my-mac" },
  }));
  const watchSessions = vi.fn(async () => ({ status: "ok", data: [] as unknown[] }));
  const notifyNeedsAttention = vi.fn(async () => {});
  const setSessionModel = vi.fn(async () => ({ status: "ok", data: null }));
  const setSessionEffort = vi.fn(async () => ({ status: "ok", data: null }));
  return {
    setSessionModel,
    setSessionEffort,
    notifyNeedsAttention,
    connectionStatus,
    listSessionTurns,
    getClientIdentity,
    watchSessions,
    FakeChannel,
    state,
    startStageSession,
    sendUserMessage,
    RESUMED,
    resumeSession,
    interruptSession,
    killSession,
    respondPermission,
    answerQuestion,
    getSessionLog,
    listActiveSessions,
    restartSession,
  };
});

vi.mock("@tauri-apps/api/core", () => ({ Channel: h.FakeChannel }));
vi.mock("@tauri-apps/api/window", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  getCurrentWindow: () => ({ isFocused: async () => false, onFocusChanged: async () => () => {} }),
}));
vi.mock("@/lib/notifications", async (importOriginal) => ({
  ...(await importOriginal<object>()),
  findWorkstreamForNotify: () => ({ projectPath: "/p", title: "ws" }),
  notifyNeedsAttention: h.notifyNeedsAttention,
}));
vi.mock("../../lib/bindings", () => ({
  commands: {
    startStageSession: h.startStageSession,
    sendUserMessage: h.sendUserMessage,
    resumeSession: h.resumeSession,
    interruptSession: h.interruptSession,
    killSession: h.killSession,
    respondPermission: h.respondPermission,
    answerQuestion: h.answerQuestion,
    getSessionLog: h.getSessionLog,
    listActiveSessions: h.listActiveSessions,
    restartSession: h.restartSession,
    connectionStatus: h.connectionStatus,
    listSessionTurns: h.listSessionTurns,
    getClientIdentity: h.getClientIdentity,
    watchSessions: h.watchSessions,
    setSessionModel: h.setSessionModel,
    setSessionEffort: h.setSessionEffort,
  },
}));

const {
  activeWorkstreamIds,
  agentStillWorking,
  allowForSession,
  answerQuestion,
  endSession,
  reattachSessions,
  restoreSessionIfNeeded,
  startSession,
  sendMessage,
  cancelQueuedMessage,
  stopSession,
  resumeSession,
  applyModel,
  applyEffort,
  applyPermissionMode,
  classifyEntry,
  workstreamShipBusy,
  dropSession,
  useSessionStore,
  useWorkstreamArtifacts,
  useMostRecentDoc,
  setSessionGoal,
  armResumeAfterSend,
  takeArmedResume,
  reattachRunningGoal,
} = await import("./sessionStore");
const { isBusy, isTerminal, isWorking } = await import("./transcript");
const { pullNow } = await import("./sessionStore");
const { forgetSyncSupport, noteComposerInput, syncInput, syncOf, useTranscriptSync } = await import("./syncStore");
const { log } = await import("@/lib/log");

const INIT: SessionEvent = { kind: "init", claudeSessionId: "c1", model: "m" };
const TURN_DONE: SessionEvent = {
  kind: "turnDone",
  costUsd: null,
  numTurns: null,
  contextTokens: null,
};

function item(event: SessionEvent) {
  return { seq: null, rev: null, author: null, event };
}

function emit(event: SessionEvent): void {
  h.state.lastChannel?.onmessage?.(item(event));
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
  h.respondPermission.mockClear();
  h.answerQuestion.mockClear();
  h.getSessionLog.mockClear();
  h.getSessionLog.mockResolvedValue({ status: "ok", data: [] });
  h.restartSession.mockClear();
  h.listSessionTurns.mockReset();
  h.state.lastChannel = null;
  forgetSyncSupport();
});

afterEach(cleanup);

describe("dropSession (transcript eviction)", () => {
  it("releases the entry and a late channel event can't resurrect it", async () => {
    const id = await startBusySession();
    emit(INIT);
    expect(useSessionStore.getState().sessions[id]).toBeDefined();

    dropSession(id);
    expect(useSessionStore.getState().sessions[id]).toBeUndefined();

    emit(TURN_DONE);
    expect(useSessionStore.getState().sessions[id]).toBeUndefined();
  });

  it("is a no-op for an unknown session id", () => {
    expect(() => dropSession("never-existed")).not.toThrow();
    expect(useSessionStore.getState().sessions).toEqual({});
  });
});

describe("conversation steering (mid-turn message queue)", () => {
  it("queues a message sent while a turn is in flight instead of sending it", async () => {
    const id = await startBusySession();
    await sendMessage(id, "follow-up");

    expect(queued(id)).toEqual(["follow-up"]);
    expect(h.sendUserMessage).not.toHaveBeenCalled();
  });

  it("keeps a fresh turn busy when init only reports process readiness", async () => {
    const id = await startBusySession();
    emit(INIT);

    expect(isWorking(entryOf(id))).toBe(true);
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
    emit(TURN_DONE);
    await Promise.resolve();
    expect(h.sendUserMessage).toHaveBeenLastCalledWith(id, "A", [], null, expect.any(String), null);
    expect(queued(id)).toEqual(["B"]);

    emit(TURN_DONE);
    await Promise.resolve();
    expect(h.sendUserMessage).toHaveBeenLastCalledWith(id, "B", [], null, expect.any(String), null);
    expect(queued(id)).toEqual([]);
  });

  it("holds queued messages while the session is parked on a permission", async () => {
    const id = await startBusySession();
    await sendMessage(id, "later");

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
      emit(INIT);

      const done = stopSession(id, () => {});
      expect(h.interruptSession).toHaveBeenCalledWith(id);
      emit(TURN_DONE);
      await vi.runAllTimersAsync();
      await done;

      expect(h.killSession).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });

  it("escalates to killSession when claude doesn't ack within the grace window", async () => {
    vi.useFakeTimers();
    try {
      const id = await startBusySession();
      emit(INIT);

      const done = stopSession(id, () => {});
      expect(h.interruptSession).toHaveBeenCalledWith(id);
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
      h.interruptSession.mockRejectedValueOnce(new Error("stdin closed"));
      const id = await startBusySession();
      emit(INIT);

      const done = stopSession(id, () => {});
      await vi.runAllTimersAsync();
      await done;

      expect(h.killSession).toHaveBeenCalledWith(id);
    } finally {
      vi.useRealTimers();
    }
  });
});


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

describe("taskStartedAt, per-task elapsed timestamps", () => {

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


const entryOf = (id: string) => useSessionStore.getState().sessions[id]!;

const QUESTION_ASK: SessionEvent = {
  kind: "questionAsk",
  requestId: "q-1",
  questions: [
    { question: "Which?", header: "Pick", multiSelect: false, options: [{ label: "A", description: "" }] },
  ],
};


describe("parked-turn recovery (permission mode change / stop)", () => {
  const PERMISSION_ASK: SessionEvent = {
    kind: "permissionAsk",
    requestId: "req-1",
    toolName: "Bash",
    input: "{}",
  };
  it("changing the permission mode while parked on a permission un-sticks the session", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(PERMISSION_ASK);
    expect(isBusy(entryOf(id))).toBe(true);

    await applyPermissionMode(id, "bypassPermissions");
    await Promise.resolve();

    expect(h.restartSession).toHaveBeenCalledWith(id, null, null, "bypassPermissions", expect.anything());
    const entry = entryOf(id);
    expect(entry.pendingPermission).toBeNull();
    expect(entry.items.some((it) => it.kind === "permission" && it.decision === null)).toBe(false);
    expect(isBusy(entry)).toBe(false);
  });

  it("auto-sends queued follow-ups after a permission-mode restart", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(PERMISSION_ASK);
    await sendMessage(id, "do it now");
    expect(queued(id)).toEqual(["do it now"]);
    expect(h.sendUserMessage).not.toHaveBeenCalled();

    await applyPermissionMode(id, "acceptEdits");
    await Promise.resolve();

    expect(h.sendUserMessage).toHaveBeenCalledWith(id, "do it now", [], null, expect.any(String), null);
    expect(queued(id)).toEqual([]);
  });

  it("switching the model on a live session resumes it on the new model", async () => {
    const id = await startBusySession();
    emit(INIT);

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

  it("a model pick on an exited tab waits on the session for its next resume", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(EXITED);
    h.setSessionModel.mockClear();
    h.setSessionEffort.mockClear();

    await applyModel(id, "sonnet", "high", "default");

    expect(h.restartSession).not.toHaveBeenCalled();
    expect(h.setSessionModel.mock.calls).toEqual([[id, "sonnet"]]);
    expect(h.setSessionEffort.mock.calls).toEqual([[id, "high"]]);
  });

  it("an effort pick waits on the session for its next launch", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(EXITED);
    h.setSessionEffort.mockClear();

    await applyEffort(id, "low");

    expect(h.restartSession).not.toHaveBeenCalled();
    expect(h.setSessionEffort.mock.calls).toEqual([[id, "low"]]);
  });

  it("stop while parked on a permission declines the live ask, interrupts, and unlocks", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(PERMISSION_ASK);
    expect(isBusy(entryOf(id))).toBe(true);

    await stopSession(id, () => {});

    expect(h.respondPermission).toHaveBeenCalledWith(id, "req-1", false, null);
    expect(h.interruptSession).toHaveBeenCalledWith(id);
    expect(h.killSession).not.toHaveBeenCalled();

    const entry = entryOf(id);
    expect(entry.pendingPermission).toBeNull();
    expect(isBusy(entry)).toBe(false);
  });

  function emitParallelAsks(): void {
    for (const [req, tool] of [
      ["r1", "getJiraIssue"],
      ["r2", "getJiraIssue"],
      ["r3", "Bash"],
    ] as const) {
      emit({ kind: "toolUse", id: `t-${req}`, name: tool, input: "{}", parentId: null });
      emit({ kind: "permissionAsk", requestId: req, toolName: tool, input: "{}", toolUseId: `t-${req}` });
    }
  }

  it("allow for this agent answers every open ask for that tool, and only that tool", async () => {
    const id = await startBusySession();
    emit(INIT);
    emitParallelAsks();

    await allowForSession(id, "r1", "getJiraIssue");

    expect(h.respondPermission).toHaveBeenCalledWith(id, "r1", true, null);
    expect(h.respondPermission).toHaveBeenCalledWith(id, "r2", true, null);
    expect(h.respondPermission).not.toHaveBeenCalledWith(id, "r3", expect.anything(), expect.anything());
    expect(entryOf(id).state).toBe("awaiting_permission");
  });

  it("stop while parked on parallel asks declines every one of them", async () => {
    const id = await startBusySession();
    emit(INIT);
    emitParallelAsks();

    await stopSession(id, () => {});

    for (const req of ["r1", "r2", "r3"]) {
      expect(h.respondPermission).toHaveBeenCalledWith(id, req, false, null);
    }
    expect(isBusy(entryOf(id))).toBe(false);
  });

  it("stop while parked on a question dismisses it, interrupts, and unlocks", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit({ kind: "toolUse", id: "tu-q", name: "AskUserQuestion", input: "{}", parentId: null });
    emit(QUESTION_ASK);
    expect(entryOf(id).pendingQuestion).not.toBeNull();

    await stopSession(id, () => {});

    expect(h.answerQuestion).toHaveBeenCalledWith(id, "q-1", {});
    expect(h.interruptSession).toHaveBeenCalledWith(id);
    expect(h.respondPermission).not.toHaveBeenCalled();

    const entry = entryOf(id);
    expect(entry.pendingQuestion).toBeNull();
    expect(isBusy(entry)).toBe(false);
  });
});


const EXITED: SessionEvent = { kind: "exited", outcome: "crashed", code: 1, stderrTail: "" };

describe("resuming a silent process settles out of `starting`", () => {
  it("a silent resume settles to a resting state instead of pulsing 'Connecting'", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(TURN_DONE);
    emit(EXITED);

    await resumeSession(id);

    const entry = entryOf(id);
    expect(entry.state).not.toBe("starting");
    expect(isWorking(entry)).toBe(false);
    expect(classifyEntry(entry)).not.toBe("working");
  });

  it("flushes a queued follow-up after a silent resume", async () => {
    const id = await startBusySession();
    await sendMessage(id, "later");
    expect(queued(id)).toEqual(["later"]);
    emit(EXITED);

    await resumeSession(id);

    expect(h.sendUserMessage).toHaveBeenCalledWith(id, "later", [], null, expect.any(String), null);
    expect(queued(id)).toEqual([]);
  });

  it("keeps an optimistic detached turn busy across a silent resume", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(TURN_DONE);
    emit(EXITED);

    await sendMessage(id, "new turn");

    expect(h.sendUserMessage).toHaveBeenCalledWith(id, "new turn", [], null, expect.any(String), null);
    expect(isWorking(entryOf(id))).toBe(true);
    expect(isBusy(entryOf(id))).toBe(true);

    await sendMessage(id, "follow-up");
    expect(h.sendUserMessage).toHaveBeenCalledTimes(1);
    expect(queued(id)).toEqual(["follow-up"]);
  });

  const resumeEmitting = (...burst: SessionEvent[]) =>
    h.resumeSession.mockImplementationOnce(async (...args: unknown[]) => {
      const ch = args[args.length - 1] as InstanceType<typeof h.FakeChannel>;
      for (const event of burst) ch.onmessage?.(item(event));
      return { status: "ok", data: h.RESUMED };
    });

  it("a resume whose only sign of life is a bare `init` settles instead of pulsing", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(TURN_DONE);
    emit(EXITED);

    resumeEmitting(INIT);
    await resumeSession(id);

    const entry = entryOf(id);
    expect(isWorking(entry)).toBe(false);
    expect(isBusy(entry)).toBe(false);
  });

  it("flushes a queued follow-up after a bare-`init` resume", async () => {
    const id = await startBusySession();
    await sendMessage(id, "later");
    emit(EXITED);

    resumeEmitting(INIT);
    await resumeSession(id);

    expect(h.sendUserMessage).toHaveBeenCalledWith(id, "later", [], null, expect.any(String), null);
    expect(queued(id)).toEqual([]);
  });

  it("does not clobber a resume that re-attaches onto a genuinely mid-turn process", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(TURN_DONE);
    emit(EXITED);

    h.resumeSession.mockImplementationOnce(async (...args: unknown[]) => {
      const ch = args[args.length - 1] as InstanceType<typeof h.FakeChannel>;
      ch.onmessage?.(item({ kind: "partialText", text: "thinking" } as SessionEvent));
      return { status: "ok", data: h.RESUMED };
    });

    await resumeSession(id);

    expect(isWorking(entryOf(id))).toBe(true);
  });
});

describe("resuming after a rewind moved the log under the cursor", () => {
  const EPOCH = { status: "error", error: { code: "session_epoch_advanced", message: "stale cursor" } };
  const LOG_DOWN = { status: "error", error: { code: "log_unavailable", message: "log read failed" } };
  const row = (seq: number, event: SessionEvent) => ({
    seq,
    kind: event.kind,
    event,
    createdAt: 1_700_000_000_000 + seq,
  });

  async function exitedSession(): Promise<string> {
    const id = await startBusySession();
    emit(INIT);
    emit(TURN_DONE);
    emit(EXITED);
    return id;
  }

  it("re-folds once and retries from the fresh log's cursor", async () => {
    const id = await exitedSession();
    h.resumeSession.mockResolvedValueOnce(EPOCH as never);
    h.getSessionLog
      .mockResolvedValueOnce({ status: "ok", data: [row(0, INIT)] })
      .mockResolvedValueOnce({ status: "ok", data: [row(0, INIT), row(1, TURN_DONE)] });

    await resumeSession(id);

    expect(h.resumeSession).toHaveBeenCalledTimes(2);
    expect((h.resumeSession.mock.calls[1] as unknown[] | undefined)?.[4]).toEqual({
      kind: "after",
      seq: 1,
      createdAt: 1_700_000_000_001,
    });
    expect(entryOf(id).live).toBe(true);
  });

  it("rolls back and stays resumable when the re-fold's log read fails", async () => {
    const id = await exitedSession();
    h.resumeSession.mockResolvedValueOnce(EPOCH as never);
    h.getSessionLog
      .mockResolvedValueOnce({ status: "ok", data: [row(0, INIT)] })
      .mockResolvedValueOnce(LOG_DOWN as never);

    await expect(resumeSession(id)).rejects.toMatchObject({ code: "log_unavailable" });

    expect(entryOf(id).live).toBe(false);
    expect(entryOf(id).state).toBe("idle");
    await resumeSession(id);
    expect(h.resumeSession).toHaveBeenCalledTimes(2);
    expect(entryOf(id).live).toBe(true);
  });

  it("gives up after a second refusal instead of retrying again", async () => {
    const id = await exitedSession();
    h.resumeSession.mockResolvedValueOnce(EPOCH as never).mockResolvedValueOnce(EPOCH as never);

    await expect(resumeSession(id)).rejects.toMatchObject({ code: "session_epoch_advanced" });

    expect(h.resumeSession).toHaveBeenCalledTimes(2);
    expect(entryOf(id).live).toBe(false);
    expect(entryOf(id).state).toBe("idle");
  });

  it("does not re-fold for any other failure", async () => {
    const id = await exitedSession();
    h.resumeSession.mockResolvedValueOnce(LOG_DOWN as never);

    await expect(resumeSession(id)).rejects.toMatchObject({ code: "log_unavailable" });

    expect(h.getSessionLog).toHaveBeenCalledTimes(1);
    expect(entryOf(id).live).toBe(false);
  });

  it("returns quietly when the tab is dropped during the re-fold", async () => {
    const id = await exitedSession();
    h.resumeSession.mockResolvedValueOnce(EPOCH as never);
    h.getSessionLog
      .mockResolvedValueOnce({ status: "ok", data: [row(0, INIT)] })
      .mockImplementationOnce(async () => {
        dropSession(id);
        return { status: "ok", data: [row(0, INIT)] };
      });

    await resumeSession(id);

    expect(h.resumeSession).toHaveBeenCalledTimes(1);
    expect(useSessionStore.getState().sessions[id]).toBeUndefined();
  });
});


const PARTIAL = (text: string): SessionEvent =>
  ({ kind: "partialText", text, parentId: null }) as SessionEvent;
const hasStreamingBlock = (id: string) =>
  entryOf(id).items.some((it) => it.kind === "assistant" && it.streaming);

describe("turn abandonment closes the streaming caret", () => {
  it("a permission-mode restart mid-stream closes the open assistant block", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(PARTIAL("thinking…"));
    expect(hasStreamingBlock(id)).toBe(true);

    await applyPermissionMode(id, "bypassPermissions");
    await Promise.resolve();

    expect(hasStreamingBlock(id)).toBe(false);
    expect(entryOf(id).streaming).toBe(false);
  });

  it("a crash mid-stream closes the open assistant block", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(PARTIAL("thinking…"));
    expect(hasStreamingBlock(id)).toBe(true);

    emit(EXITED);

    expect(hasStreamingBlock(id)).toBe(false);
  });
});

describe("classifyEntry, background activity state", () => {
  it("returns 'background' for a live resting session with a running background task", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(BASH_BG_SPAWN("tu-1", "task-a"));
    emit(BASH_BG_RESULT("tu-1", "task-a"));
    emit(TURN_DONE);

    const entry = entryOf(id);
    expect(entry.state).toBe("running");
    expect(entry.streaming).toBe(false);
    expect(classifyEntry(entry)).toBe("background");
  });

  it("returns 'review' (not 'background') for a DETACHED session (live:false)", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(BASH_BG_SPAWN("tu-1", "task-a"));
    emit(BASH_BG_RESULT("tu-1", "task-a"));
    emit(TURN_DONE);

    const live = entryOf(id);
    expect(classifyEntry(live)).toBe("background");
    expect(classifyEntry({ ...live, live: false })).toBe("review");
  });

  it("flips from 'background' to 'review' once the task completes via notification", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(BASH_BG_SPAWN("tu-1", "task-a"));
    emit(BASH_BG_RESULT("tu-1", "task-a"));
    emit(TURN_DONE);
    expect(classifyEntry(entryOf(id))).toBe("background");

    emit({ kind: "backgroundTaskUpdate", taskId: "task-a", toolUseId: "tu-1", status: "completed" });

    const entry = entryOf(id);
    expect(entry.state).toBe("running");
    expect(entry.streaming).toBe(false);
    expect(classifyEntry(entry)).toBe("review");
  });

  it("stays 'background' while a workflow's parented events keep arriving", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit({
      kind: "toolUse",
      id: "wf-1",
      name: "Workflow",
      input: JSON.stringify({ script: "export const meta = {…", description: "map the store" }),
      parentId: null,
    });
    emit({
      kind: "toolResult",
      toolUseId: "wf-1",
      content: "Workflow launched in background. Task ID: whjv5o0xq\nRun ID: wf_1",
      isError: false,
      parentId: null,
    });
    emit(TURN_DONE);
    expect(classifyEntry(entryOf(id))).toBe("background");

    emit({
      kind: "toolUse",
      id: "wf:wf_1:agent1",
      name: "Agent",
      input: JSON.stringify({ description: "map the stage model" }),
      parentId: "wf-1",
    });
    emit({
      kind: "toolUse",
      id: "inner-1",
      name: "Grep",
      input: JSON.stringify({ pattern: "foo" }),
      parentId: "wf:wf_1:agent1",
    });

    const entry = entryOf(id);
    expect(entry.streaming).toBe(false);
    expect(classifyEntry(entry)).toBe("background");
  });

  it("returns 'working' (which outranks background) while the turn is streaming", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(BASH_BG_SPAWN("tu-1", "task-a"));
    emit(BASH_BG_RESULT("tu-1", "task-a"));
    const entry = entryOf(id);
    expect(entry.streaming).toBe(true);
    expect(classifyEntry(entry)).toBe("working");
  });

  it("returns 'review' (not 'background') for a COMPLETED session even with an un-stopped task", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(BASH_BG_SPAWN("tu-1", "task-a"));
    emit(BASH_BG_RESULT("tu-1", "task-a"));
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

  it("a DETACHED session drops its live flag but stays non-terminal", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(BASH_BG_SPAWN("tu-1", "task-a"));
    emit({ kind: "exited", outcome: "detached", code: null, stderrTail: "" });

    const entry = entryOf(id);
    expect(entry.state).toBe("idle");
    expect(entry.live).toBe(false);
    expect(entry.items.some((it) => it.kind === "exit")).toBe(false);
  });
});


describe("workstreamShipBusy", () => {
  const shipBusy = (ws: string) => workstreamShipBusy(useSessionStore.getState().sessions, ws);

  it("is true while the ship session is mid-turn, false once that turn parks", async () => {
    await startSession({ workstreamId: "ws1", stage: "ship", input: "" });
    expect(shipBusy("ws1")).toBe(true);

    emit(INIT);
    expect(shipBusy("ws1")).toBe(true);

    emit(TURN_DONE);
    expect(shipBusy("ws1")).toBe(false);
  });

  it("stays true while the ship turn is paused on a permission (needs-you)", async () => {
    await startSession({ workstreamId: "ws1", stage: "ship", input: "" });
    emit(INIT);
    emit({ kind: "permissionAsk", requestId: "req-1", toolName: "Bash", input: "{}" });
    expect(shipBusy("ws1")).toBe(true);
  });

  it("ignores non-ship sessions and other workstreams' ship sessions", async () => {
    await startSession({ workstreamId: "ws1", stage: "general", input: "" });
    expect(shipBusy("ws1")).toBe(false);

    await startSession({ workstreamId: "ws2", stage: "ship", input: "" });
    expect(shipBusy("ws1")).toBe(false);
    expect(shipBusy("ws2")).toBe(true);
  });
});

describe("useWorkstreamArtifacts", () => {
  function publish(toolUseId: string, filePath: string, url: string) {
    emit({ kind: "toolUse", id: toolUseId, name: "Artifact", input: JSON.stringify({ file_path: filePath }), parentId: null });
    emit({
      kind: "toolResult",
      toolUseId,
      content: `Published ${filePath} at ${url}`,
      isError: false,
      parentId: null,
    });
  }

  it("shows an artifact published in a session bound to the workstream", async () => {
    await startSession({ workstreamId: "ws1", stage: "general", input: "go" });
    publish("tu1", "/tmp/a.html", "https://claude.ai/code/artifact/abc");

    const { result } = renderHook(() => useWorkstreamArtifacts("ws1"));
    expect(result.current.map((a) => a.url)).toEqual(["https://claude.ai/code/artifact/abc"]);
  });

  it("ignores an artifact from a session bound to a different workstream", async () => {
    await startSession({ workstreamId: "ws2", stage: "general", input: "go" });
    publish("tu1", "/tmp/a.html", "https://claude.ai/code/artifact/abc");

    const { result } = renderHook(() => useWorkstreamArtifacts("ws1"));
    expect(result.current).toEqual([]);
  });

  it("collapses the same URL published from two sessions of the same workstream into one entry", async () => {
    await startSession({ workstreamId: "ws1", stage: "general", input: "go" });
    publish("tu1", "/tmp/a.html", "https://claude.ai/code/artifact/abc");

    await startSession({ workstreamId: "ws1", stage: "general", input: "go again" });
    publish("tu2", "/tmp/a-renamed.html", "https://claude.ai/code/artifact/abc");

    const { result } = renderHook(() => useWorkstreamArtifacts("ws1"));
    expect(result.current).toHaveLength(1);
    expect(result.current[0].filePath).toBe("/tmp/a-renamed.html");
  });
});

describe("activeWorkstreamIds", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("includes a workstream with a session currently working", async () => {
    await startSession({ workstreamId: "ws1", stage: "general", input: "go" });
    expect(activeWorkstreamIds()).toContain("ws1");
  });

  it("excludes a workstream idle past the attribution grace window", async () => {
    vi.useFakeTimers();
    await startSession({ workstreamId: "ws1", stage: "general", input: "go" });
    emit(INIT);
    emit(TURN_DONE);
    vi.advanceTimersByTime(31_000);
    expect(activeWorkstreamIds()).not.toContain("ws1");
  });

  it("still includes a workstream that stopped working seconds ago", async () => {
    vi.useFakeTimers();
    await startSession({ workstreamId: "ws1", stage: "general", input: "go" });
    emit(INIT);
    emit(TURN_DONE);
    vi.advanceTimersByTime(5_000);
    expect(activeWorkstreamIds()).toContain("ws1");
  });

});

describe("useMostRecentDoc", () => {
  function toolUse(id: string, name: string, input: Record<string, unknown>): SessionEvent {
    return { kind: "toolUse", id, name, input: JSON.stringify(input), parentId: null };
  }

  it("returns the plan a workstream's own session wrote", async () => {
    await startSession({ workstreamId: "ws1", stage: "general", input: "go" });
    emit(toolUse("tu1", "Write", { file_path: "/t/shared/plans/2026-08-21-a.md" }));

    const { result } = renderHook(() => useMostRecentDoc("ws1", "plan"));
    expect(result.current?.path).toBe("/t/shared/plans/2026-08-21-a.md");
  });

  it("returns null for a workstream whose only reference to the plan is a read", async () => {
    await startSession({ workstreamId: "ws1", stage: "general", input: "go" });
    emit(toolUse("tu1", "Write", { file_path: "/t/shared/plans/2026-08-21-a.md" }));

    await startSession({ workstreamId: "ws2", stage: "general", input: "go" });
    emit(toolUse("tu2", "Bash", { command: "cat /t/shared/plans/2026-08-21-a.md" }));

    const { result } = renderHook(() => useMostRecentDoc("ws2", "plan"));
    expect(result.current).toBeNull();
  });
});

describe("restoring a stored session log", () => {
  type LogEntry = { seq: number; kind: string; event: unknown; createdAt: number };

  function entry(seq: number, event: unknown, kind?: string): LogEntry {
    return {
      seq,
      kind: kind ?? (event as { kind: string }).kind,
      event,
      createdAt: 1_700_000_000_000 + seq,
    };
  }

  let restoreSeq = 0;

  // A restore that resumes needs its own id: the channel outlives the store reset.
  async function restoreLog(
    log: LogEntry[],
    opts: { live?: boolean; id?: string } = {},
  ): Promise<ReturnType<typeof entryOf>> {
    const live = opts.live ?? false;
    const id = opts.id ?? (live ? `restored-${(restoreSeq += 1)}` : "stored-1");
    h.getSessionLog.mockResolvedValue({ status: "ok", data: log });
    if (live) h.listActiveSessions.mockResolvedValueOnce({ status: "ok", data: [{ session: { id } }] });
    await restoreSessionIfNeeded(id, "ws1", "general");
    return entryOf(id);
  }

  it("attaches to a restored session the server still reports live", async () => {
    // Its own id: the resume registers a channel that outlives the store reset.
    h.getSessionLog.mockResolvedValueOnce({ status: "ok", data: [entry(0, INIT)] });
    h.listActiveSessions.mockResolvedValueOnce({
      status: "ok",
      data: [{ session: { id: "live-1" }, workstreamTitle: "Test", projectPath: "/p" }],
    });
    await restoreSessionIfNeeded("live-1", "ws1", "general");
    const restored = entryOf("live-1");
    expect(h.resumeSession).toHaveBeenCalledWith(
      "live-1",
      null,
      null,
      null,
      { kind: "after", seq: 0, createdAt: entry(0, INIT).createdAt },
      expect.anything(),
    );
    expect(restored.live).toBe(true);
    expect(isTerminal(restored.state)).toBe(false);
  });

  it("leaves a restored session parked when the server does not report it live", async () => {
    const restored = await restoreLog([entry(0, INIT)]);
    expect(h.resumeSession).not.toHaveBeenCalled();
    expect(restored.live).toBe(false);
  });

  const ASK_LOG = () => [
    entry(0, INIT),
    entry(1, { kind: "toolUse", id: "t1", name: "Bash", input: "{}", parentId: null }),
    entry(2, { kind: "permissionAsk", requestId: "req-1", toolName: "Bash", input: "{}" }),
  ];

  it("restores a dead session parked on an approval as expired, not busy", async () => {
    const restored = await restoreLog(ASK_LOG());
    expect(restored.live).toBe(false);
    expect(restored.state).toBe("idle");
    expect(isBusy(restored)).toBe(false);
    expect(classifyEntry(restored)).not.toBe("needs-you");
    expect(restored.items.find((it) => it.kind === "permission")).toMatchObject({ decision: "expired" });
  });

  it("continues an expired session with one resume and one send, never the stale decision", async () => {
    await restoreLog(ASK_LOG(), { id: "expired-continue" });

    await sendMessage("expired-continue", "carry on");

    expect(h.resumeSession).toHaveBeenCalledTimes(1);
    expect(h.sendUserMessage).toHaveBeenCalledTimes(1);
    expect(h.sendUserMessage).toHaveBeenCalledWith("expired-continue", "carry on", [], null, expect.any(String), null);
    expect(h.respondPermission).not.toHaveBeenCalled();
  });

  it("restores a live session parked on an approval with its card answerable", async () => {
    const restored = await restoreLog(ASK_LOG(), { live: true });
    expect(restored.live).toBe(true);
    expect(restored.state).toBe("awaiting_permission");
    expect(isBusy(restored)).toBe(true);
    expect(restored.items.find((it) => it.kind === "permission")).toMatchObject({ decision: null });
  });

  it("replays a typed question array straight into the pending question", async () => {
    const restored = await restoreLog([entry(0, INIT), entry(1, QUESTION_ASK)], { live: true });
    expect(restored.pendingQuestion).toEqual({
      requestId: "q-1",
      questions: [
        { question: "Which?", header: "Pick", multiSelect: false, options: [{ label: "A", description: "" }] },
      ],
    });
  });

  it("normalizes a legacy string-encoded question row from an older build", async () => {
    const restored = await restoreLog([
      entry(0, INIT),
      entry(1, {
        kind: "questionAsk",
        requestId: "q-1",
        questions: JSON.stringify([
          { question: "Which?", header: "Pick", multiSelect: false, options: [{ label: "A", description: "" }] },
        ]),
      }),
    ], { live: true });
    expect(restored.pendingQuestion?.questions).toEqual([
      { question: "Which?", header: "Pick", multiSelect: false, options: [{ label: "A", description: "" }] },
    ]);
  });

  it("parks with no questions when the stored row's questions are missing or malformed", async () => {
    for (const questions of [undefined, "not json{", { not: "an array" }]) {
      useSessionStore.setState({ sessions: {} });
      const restored = await restoreLog([
        entry(0, INIT),
        entry(1, { kind: "questionAsk", requestId: "q-1", questions }),
      ], { live: true });
      expect(restored.pendingQuestion).toEqual({ requestId: "q-1", questions: [] });
    }
  });

  it("skips a stored row the reducer does not recognize instead of dropping the log", async () => {
    const restored = await restoreLog([
      entry(0, INIT),
      entry(1, { kind: "toolPermissionEscalated", requestId: "future-1" }, "toolPermissionEscalated"),
      entry(2, { kind: "assistantText", text: "still here", parentId: null, injected: false }),
    ]);
    expect(restored.items.map((it) => it.kind)).toEqual(["assistant"]);
  });

  it("stamps a stored user message with the row's createdAt", async () => {
    const restored = await restoreLog([
      entry(0, INIT),
      entry(1, { kind: "userMessage", text: "hello", stage: null, promptName: null }),
    ]);
    const first = restored.items[0];
    expect(first.kind).toBe("user");
    expect(first.kind === "user" && first.createdAt).toBe(1_700_000_000_001);
  });
});

describe("archiving or closing an agent", () => {
  const ASK: SessionEvent = { kind: "permissionAsk", requestId: "req-1", toolName: "Bash", input: "{}" };
  const KILLED: SessionEvent = { kind: "exited", outcome: "killed", code: null, stderrTail: "" };

  it("lets an agent waiting on you be archived, but not one still working", async () => {
    const id = await startBusySession();
    expect(agentStillWorking(entryOf(id))).toBe(true);
    emit(INIT);
    emit(ASK);
    expect(agentStillWorking(entryOf(id))).toBe(false);
  });

  it("keeps an idle agent with background work from being archived", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit({ kind: "toolUse", id: "bg", name: "Bash", input: '{"run_in_background":true}', parentId: null });
    emit({ kind: "toolResult", toolUseId: "bg", content: "Command running in background with ID: b1", isError: false, parentId: null });
    emit(TURN_DONE);
    expect(isWorking(entryOf(id))).toBe(false);
    expect(classifyEntry(entryOf(id))).toBe("background");
    expect(agentStillWorking(entryOf(id))).toBe(true);
  });

  it("ends a parked agent's harness, expiring its open ask when the exit lands", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(ASK);

    await endSession(id);
    emit(KILLED);

    expect(h.killSession).toHaveBeenCalledWith(id);
    const entry = entryOf(id);
    expect(entry.items.find((it) => it.kind === "permission")).toMatchObject({ decision: "expired" });
    expect(isBusy(entry)).toBe(false);
    expect(h.respondPermission).not.toHaveBeenCalled();
  });

  it("ends a session the server runs but this window never attached", async () => {
    h.listActiveSessions.mockResolvedValueOnce({ status: "ok", data: [{ session: { id: "unattached" } }] });
    await endSession("unattached");
    expect(h.killSession).toHaveBeenCalledWith("unattached");
    expect(useSessionStore.getState().sessions.unattached).toBeUndefined();
  });

  it("leaves a session that already ended alone", async () => {
    await restoreSessionIfNeeded("ended-1", "ws1", "general");
    const before = entryOf("ended-1");
    await endSession("ended-1");
    expect(h.killSession).not.toHaveBeenCalled();
    expect(entryOf("ended-1")).toBe(before);
  });

  it("does not throw when the server refuses the kill", async () => {
    h.listActiveSessions.mockResolvedValueOnce({ status: "ok", data: [{ session: { id: "refused" } }] });
    h.killSession.mockResolvedValueOnce({ status: "error", error: { code: "session_not_found" } } as never);
    await expect(endSession("refused")).resolves.toBeUndefined();
  });
});

describe("answering a question", () => {
  it("hands the answer record to the command without stringifying it", async () => {
    const id = await startBusySession();
    emit(INIT);
    emit(QUESTION_ASK);

    await answerQuestion(id, "q-1", { "Which?": "A" });

    expect(h.answerQuestion).toHaveBeenCalledWith(id, "q-1", { "Which?": "A" });
    expect(entryOf(id).pendingQuestion).toBeNull();
  });
});

describe("transcript sync: queue and reload", () => {
  const SELF = { clientId: "me", device: "my-mac" };
  const OTHER = { clientId: "them", device: "jons-macbook-pro" };
  const THEIRS: SessionEvent = { kind: "userMessage", text: "theirs", stage: null, author: OTHER };
  const reply = (text: string): SessionEvent => ({ kind: "assistantText", text, parentId: null });
  const row = (seq: number, event: SessionEvent) => ({ seq, kind: event.kind, event, createdAt: 1_700_000_000_000 + seq });
  type Row = ReturnType<typeof row>;
  const page = (entries: Row[], rev: number, over: Record<string, unknown> = {}) => {
    const last = entries[entries.length - 1];
    return {
      status: "ok",
      data: {
        entries,
        nextCursor: last ? { kind: "after", seq: last.seq, createdAt: last.createdAt } : { kind: "empty" },
        remaining: 0,
        rev,
        resetRev: 0,
        working: false,
        inFlight: 0,
        ...over,
      },
    };
  };
  const STALE = (rev: number, committedRev: number) => ({
    status: "error",
    error: {
      code: "stale_transcript",
      message: "this session moved on",
      remedy: "Pull the new turns, then send again.",
      staleTranscript: { rev, committedRev, missingTurns: rev },
    },
  });

  function emitAt(event: SessionEvent, seq: number | null, rev: number, author: typeof SELF | null = SELF): void {
    h.state.lastChannel?.onmessage?.({ seq, rev, author, event });
  }

  const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
  const sent = () => (h.sendUserMessage.mock.calls as unknown[][]).map((c) => [c[1], c[5]]);

  beforeEach(() => {
    h.connectionStatus.mockResolvedValue({ status: "ok", data: { state: "connected", transcriptSync: true } });
    h.listSessionTurns.mockResolvedValue(page([], 0));
    useTranscriptSync.setState({ self: SELF });
  });

  afterEach(() => {
    h.connectionStatus.mockResolvedValue({ status: "ok", data: { state: "connected", transcriptSync: false } });
    useTranscriptSync.setState({ self: null });
    vi.restoreAllMocks();
  });

  it("holds another device's turn on an idle attached view until Pull", async () => {
    const id = await startBusySession();
    emitAt(INIT, 0, 0);
    emitAt(TURN_DONE, 1, 0);
    emitAt(THEIRS, 2, 1, OTHER);
    emitAt(reply("theirs done"), 3, 2, OTHER);
    emitAt(TURN_DONE, 4, 2, OTHER);
    await new Promise((resolve) => setTimeout(resolve, 60));

    const texts = () => entryOf(id).items.map((it) => ("text" in it ? it.text : it.kind));
    expect(texts()).not.toContain("theirs done");
    expect(h.listSessionTurns).not.toHaveBeenCalled();
    expect(syncOf(id)?.phase).toBe("behind");

    await pullNow(id);
    expect(texts()).toContain("theirs done");
    expect(entryOf(id).streaming).toBe(false);
    expect(syncOf(id)?.phase).toBe("inSync");
  });

  it("never alerts for a prompt in another device's turn, even when Pull applies it", async () => {
    const ask: SessionEvent = { kind: "permissionAsk", requestId: "r1", toolName: "Bash", input: "{}" };
    h.notifyNeedsAttention.mockClear();
    const id = await startBusySession();
    emitAt(INIT, 0, 0);
    emitAt(TURN_DONE, 1, 0);
    emitAt(THEIRS, 2, 1, OTHER);
    emitAt(ask, 3, 1, OTHER);
    await settle();
    await pullNow(id);
    await settle();

    expect(classifyEntry(entryOf(id))).toBe("needs-you");
    expect(h.notifyNeedsAttention).not.toHaveBeenCalled();
  });

  it("alerts for a prompt in this device's own turn", async () => {
    const ask: SessionEvent = { kind: "permissionAsk", requestId: "r2", toolName: "Bash", input: "{}" };
    h.notifyNeedsAttention.mockClear();
    await startBusySession();
    emitAt(INIT, 0, 0);
    emitAt(ask, 1, 0);
    await settle();

    expect(h.notifyNeedsAttention).toHaveBeenCalledTimes(1);
  });

  it("returns a flush refused as stale to the head of the queue and sends it again after the pull", async () => {
    const id = await startBusySession();
    emitAt(INIT, 0, 0);
    await sendMessage(id, "A");
    h.sendUserMessage.mockResolvedValueOnce(STALE(2, 2) as never);
    h.listSessionTurns.mockResolvedValueOnce(page([row(2, THEIRS), row(3, reply("theirs done")), row(4, TURN_DONE)], 2));

    emitAt(TURN_DONE, 1, 0);
    await settle();

    expect(h.listSessionTurns).toHaveBeenCalledTimes(1);
    expect(sent()).toEqual([
      ["A", 0],
      ["A", 2],
    ]);
    expect(queued(id)).toEqual([]);
    expect(syncOf(id)?.committedRev).toBe(2);
  });

  it("pulls before flushing a queued message while the view is behind", async () => {
    const id = await startBusySession();
    emitAt(INIT, 0, 0);
    await sendMessage(id, "A");
    noteComposerInput(id, "still typing");
    syncInput(id, {
      kind: "remote",
      rev: 1,
      turns: [{ rev: 1, kinds: ["user"] }],
      device: OTHER.device,
      working: false,
      flushable: false,
      attached: false,
      own: false,
    });
    expect(h.listSessionTurns).not.toHaveBeenCalled();
    h.listSessionTurns.mockResolvedValueOnce(page([row(2, THEIRS), row(3, reply("theirs done")), row(4, TURN_DONE)], 2));

    emitAt(TURN_DONE, 1, 0);
    await settle();

    expect(h.listSessionTurns).toHaveBeenCalledTimes(1);
    expect(h.listSessionTurns.mock.invocationCallOrder[0]).toBeLessThan(h.sendUserMessage.mock.invocationCallOrder[0]);
    expect(sent()).toEqual([["A", 2]]);
  });

  it("queues instead of sending when the resume attached mid-turn", async () => {
    const id = await startBusySession();
    emitAt(INIT, 0, 0);
    emitAt(TURN_DONE, 1, 0);
    emitAt(EXITED, 2, 0);
    h.listSessionTurns.mockResolvedValueOnce(page([row(0, INIT), row(1, TURN_DONE), row(2, EXITED)], 0));
    h.resumeSession.mockImplementationOnce(async (...args: unknown[]) => {
      const ch = args[args.length - 1] as InstanceType<typeof h.FakeChannel>;
      // A replayed row: author-less, so it applies at once.
      ch.onmessage?.({ seq: 3, rev: 1, author: null, event: { ...THEIRS, author: null } });
      return { status: "ok", data: h.RESUMED };
    });

    await sendMessage(id, "mine");

    expect(h.resumeSession).toHaveBeenCalledTimes(1);
    expect(h.sendUserMessage).not.toHaveBeenCalled();
    expect(queued(id)).toEqual(["mine"]);
  });

  it("returns a flushed message to the head and a new one to the tail when a pull reveals a running turn", async () => {
    const id = await startBusySession();
    emitAt(INIT, 0, 0);
    await sendMessage(id, "A");
    await sendMessage(id, "B");
    syncInput(id, { kind: "polled", rev: 1, resetRev: 0, inFlight: 0, working: true });
    h.listSessionTurns.mockResolvedValueOnce(page([row(2, THEIRS)], 1, { working: true }));

    emitAt(TURN_DONE, 1, 0);
    await settle();
    await sendMessage(id, "C");

    expect(h.sendUserMessage).not.toHaveBeenCalled();
    expect(queued(id)).toEqual(["A", "B", "C"]);
  });

  it("logs a queued message that fails for any other reason", async () => {
    const warn = vi.spyOn(log, "warn");
    const id = await startBusySession();
    emitAt(INIT, 0, 0);
    await sendMessage(id, "A");
    await sendMessage(id, "B");
    h.sendUserMessage.mockResolvedValueOnce({
      status: "error",
      error: { code: "internal", message: "boom", remedy: null },
    } as never);

    emitAt(TURN_DONE, 1, 0);
    await settle();

    expect(warn).toHaveBeenCalledWith("session", expect.stringContaining("queued"), expect.objectContaining({ code: "internal" }));
    expect(queued(id)).toEqual(["B"]);
    expect(entryOf(id).items.some((it) => it.kind === "user" && it.text === "A")).toBe(false);
  });

  it("sends queued messages in order, once each, only after another client's in-flight send lands", async () => {
    const id = await startBusySession();
    emitAt(INIT, 0, 0);
    await sendMessage(id, "A");
    await sendMessage(id, "B");
    syncInput(id, { kind: "polled", rev: 0, resetRev: 0, inFlight: 1, working: true });
    h.listSessionTurns.mockResolvedValueOnce(page([], 0, { inFlight: 1 }));

    emitAt(TURN_DONE, 1, 0);
    await settle();

    expect(h.sendUserMessage).not.toHaveBeenCalled();
    expect(queued(id)).toEqual(["A", "B"]);

    h.listSessionTurns.mockResolvedValueOnce(page([row(2, THEIRS), row(3, reply("theirs done")), row(4, TURN_DONE)], 2));
    syncInput(id, { kind: "polled", rev: 2, resetRev: 0, inFlight: 0, working: false });
    await settle();

    expect(sent()).toEqual([["A", 2]]);
    expect(queued(id)).toEqual(["B"]);

    emitAt(TURN_DONE, 5, 2);
    await settle();

    expect(sent()).toEqual([
      ["A", 2],
      ["B", 2],
    ]);
    expect(queued(id)).toEqual([]);
    expect(h.listSessionTurns).toHaveBeenCalledTimes(2);
  });

  it("reads again when a revision lands during a pull, and settles in sync", async () => {
    const id = await startBusySession();
    emitAt(INIT, 0, 0);
    emitAt(TURN_DONE, 1, 0);
    h.listSessionTurns
      .mockImplementationOnce(async () => {
        syncInput(id, {
          kind: "remote",
          rev: 3,
          turns: [{ rev: 3, kinds: ["assistant"] }],
          device: OTHER.device,
          working: true,
          flushable: false,
          attached: false,
          own: false,
        });
        return page([row(2, THEIRS)], 2, { working: true });
      })
      .mockResolvedValueOnce(page([row(3, reply("more")), row(4, TURN_DONE)], 3));

    syncInput(id, { kind: "polled", rev: 2, resetRev: 0, inFlight: 0, working: true });
    await pullNow(id);
    await settle();

    expect(h.listSessionTurns).toHaveBeenCalledTimes(2);
    const sync = syncOf(id);
    expect(sync?.localRev).toBe(3);
    expect(sync?.fetching).toBe(false);
    expect(sync?.phase).toBe("inSync");
  });

  it("keeps a refused flush at the head while the winner's turn runs, and sends it once after that turn ends", async () => {
    const id = await startBusySession();
    emitAt(INIT, 0, 0);
    await sendMessage(id, "A2");
    // The other client's message lands mid-send; the server then refuses ours.
    h.sendUserMessage.mockImplementationOnce(async () => {
      emitAt({ ...THEIRS, text: "B wins" }, 2, 1, OTHER);
      return STALE(1, 1) as never;
    });

    emitAt(TURN_DONE, 1, 0);
    await settle();
    await settle();

    expect(sent()).toEqual([["A2", 0]]);
    expect(queued(id)).toEqual(["A2"]);
    expect(entryOf(id).streaming).toBe(true);
    expect(entryOf(id).turnAuthor).toBe("other");

    emitAt(TURN_DONE, 3, 1, OTHER);
    await settle();

    expect(sent()).toEqual([
      ["A2", 0],
      ["A2", 1],
    ]);
    expect(queued(id)).toEqual([]);
  });

  it("recovers a view left syncing when the reload its reset asked for was dropped", async () => {
    const id = await startBusySession();
    emitAt(INIT, 0, 0);
    emitAt(TURN_DONE, 1, 0);
    let finishRestart: () => void = () => {};
    h.restartSession.mockImplementationOnce(async (...args: unknown[]) => {
      h.state.lastChannel = args[args.length - 1] as InstanceType<typeof h.FakeChannel>;
      await new Promise<void>((resolve) => (finishRestart = resolve));
      return { status: "ok", data: null };
    });
    const restarting = applyPermissionMode(id, "acceptEdits");
    // A reset during a mode restart: the reload it asks for is gated off by `reloading`.
    syncInput(id, { kind: "reset" });
    finishRestart();
    await restarting;
    expect(syncOf(id)?.fetching).toBe(true);
    expect(h.listSessionTurns).not.toHaveBeenCalled();

    h.listSessionTurns.mockResolvedValueOnce(page([row(2, THEIRS), row(3, reply("theirs done")), row(4, TURN_DONE)], 2));
    syncInput(id, { kind: "polled", rev: 2, resetRev: 0, inFlight: 0, working: false });
    await settle();

    expect(h.listSessionTurns).toHaveBeenCalledTimes(1);
    expect(syncOf(id)?.localRev).toBe(2);
    expect(syncOf(id)?.fetching).toBe(false);
    expect(syncOf(id)?.phase).toBe("inSync");
  });

  it("tears down an attached stream when a pulled page shows the session exited", async () => {
    const id = await startBusySession();
    emitAt(INIT, 0, 0);
    emitAt(TURN_DONE, 1, 0);
    const COMPLETED: SessionEvent = { kind: "exited", outcome: "completed", code: 0, stderrTail: "" };
    h.listSessionTurns.mockResolvedValueOnce(page([row(2, THEIRS), row(3, reply("bye")), row(4, COMPLETED)], 2));

    syncInput(id, { kind: "polled", rev: 2, resetRev: 0, inFlight: 0, working: false });
    await pullNow(id);

    expect(h.listSessionTurns).toHaveBeenCalledTimes(1);
    expect(entryOf(id).state).toBe("completed");
    expect(entryOf(id).live).toBe(false);
    // With no channel registered, stop has nothing to interrupt.
    await stopSession(id, () => {});
    expect(h.interruptSession).not.toHaveBeenCalled();

    const before = entryOf(id);
    emitAt(COMPLETED, 4, 2, null);
    expect(entryOf(id)).toBe(before);
  });

  it("applies an item at a seq a rewind reused once the reload has lowered appliedSeq", async () => {
    const id = "rewound-1";
    const live = { status: "ok", data: [{ session: { id } }] };
    const channels: InstanceType<typeof h.FakeChannel>[] = [];
    h.resumeSession.mockImplementation(async (...args: unknown[]) => {
      channels.push(args[args.length - 1] as InstanceType<typeof h.FakeChannel>);
      return { status: "ok", data: h.RESUMED };
    });
    const one: SessionEvent = { kind: "userMessage", text: "one", stage: null };
    const two: SessionEvent = { kind: "userMessage", text: "two", stage: null };
    h.listActiveSessions.mockResolvedValueOnce(live).mockResolvedValueOnce(live);
    h.listSessionTurns
      .mockResolvedValueOnce(page([row(0, INIT), row(1, one), row(2, reply("a")), row(3, two), row(4, reply("b"))], 4))
      .mockResolvedValueOnce(
        page([row(0, INIT), row(1, one), row(2, reply("a")), row(3, { kind: "rewound" })], 5, { resetRev: 5 }),
      );

    await restoreSessionIfNeeded(id, "ws1", "general");
    expect(channels).toHaveLength(1);
    channels[0].onmessage?.({ seq: 3, rev: 5, author: null, event: { kind: "rewound" } });
    await settle();
    await settle();

    expect(channels).toHaveLength(2);
    expect(syncOf(id)?.resetRev).toBe(5);
    channels[1].onmessage?.({ seq: 4, rev: 6, author: null, event: reply("after the rewind") });

    const texts = entryOf(id).items.map((it) => ("text" in it ? it.text : it.kind));
    expect(texts).toContain("after the rewind");
    expect(texts).not.toContain("b");
    h.resumeSession.mockReset();
    h.resumeSession.mockImplementation(async () => ({ status: "ok", data: h.RESUMED }));
  });
});

describe("reconnecting from the held cursor", () => {
  const SELF = { clientId: "me", device: "my-mac" };
  const reply = (text: string): SessionEvent => ({ kind: "assistantText", text, parentId: null });
  const partial = (text: string): SessionEvent => ({ kind: "partialText", text, parentId: null });
  type Row = { seq: number; kind: string; event: SessionEvent; createdAt: number };
  const at = (seq: number, event: SessionEvent, createdAt = 1_700_000_000_000 + seq): Row => ({
    seq,
    kind: event.kind,
    event,
    createdAt,
  });
  const history = (n: number): Row[] => [at(0, INIT), ...Array.from({ length: n - 1 }, (_, i) => at(i + 1, reply(`r${i + 1}`)))];

  // The server as it is: rows after the cursor it is handed, refusing one a rewind rewrote.
  const server = new Map<string, { rows: Row[]; resetRev: number }>();
  const reads: { sessionId: string; cursor: LogCursorDto; rows: number }[] = [];
  const streams = new Map<string, InstanceType<typeof h.FakeChannel>>();
  let nextId = 0;

  function listTurns(sessionId: string, cursor: LogCursorDto, limit: number) {
    const log = server.get(sessionId)!;
    if (cursor.kind === "after" && log.rows[cursor.seq ?? -1]?.createdAt !== cursor.createdAt) {
      return { status: "error", error: { code: "session_epoch_advanced", message: "stale cursor" } };
    }
    const from = cursor.kind === "after" ? (cursor.seq ?? -1) + 1 : 0;
    const entries = log.rows.slice(from, from + limit);
    reads.push({ sessionId, cursor, rows: entries.length });
    const last = entries[entries.length - 1];
    return {
      status: "ok",
      data: {
        entries,
        nextCursor: last ? { kind: "after", seq: last.seq, createdAt: last.createdAt } : cursor,
        remaining: Math.max(0, log.rows.length - from - entries.length),
        rev: log.rows.length,
        resetRev: log.resetRev,
        working: false,
        inFlight: 0,
      },
    };
  }

  const tail = (id: string): LogCursorDto => {
    const { rows } = server.get(id)!;
    const last = rows[rows.length - 1];
    return { kind: "after", seq: last.seq, createdAt: last.createdAt };
  };
  const texts = (id: string) => entryOf(id).items.flatMap((it) => (it.kind === "assistant" ? [it.text] : []));
  const resumedFrom = () => (h.resumeSession.mock.calls as unknown[][]).map((c) => [c[0], c[4]]);
  const asked = () => (h.listSessionTurns.mock.calls as unknown[][]).map((c) => [c[0], c[1]]);
  const cursorAt = (row: Row): LogCursorDto => ({ kind: "after", seq: row.seq, createdAt: row.createdAt });
  const replies = (n: number) => Array.from({ length: n - 1 }, (_, i) => `r${i + 1}`);
  const turn = (n: number): Row[] => [...history(n), at(n, TURN_DONE)];

  // As a server from 0.8.21 streams a logged row: with the createdAt that makes it a cursor.
  const stream = (id: string, row: Row, author: typeof SELF = SELF) =>
    streams.get(id)!.onmessage?.({ seq: row.seq, createdAt: row.createdAt, rev: row.seq + 1, author, event: row.event });

  // Started in this window, so it holds no folded cursor: every row arrived live.
  async function startStreaming(rows: Row[]): Promise<string> {
    const { id } = await startSession({ workstreamId: "ws1", stage: "general", input: "kick off" });
    server.set(id, { rows, resetRev: 0 });
    streams.set(id, h.state.lastChannel!);
    for (const row of rows) stream(id, row);
    return id;
  }

  async function restore(rows: Row[], live: boolean): Promise<string> {
    const id = `reconnect-${(nextId += 1)}`;
    server.set(id, { rows, resetRev: 0 });
    h.listActiveSessions.mockResolvedValueOnce({ status: "ok", data: live ? [{ session: { id } }] : [] });
    await restoreSessionIfNeeded(id, "ws1", "general");
    return id;
  }

  async function reconnect(live: string[]): Promise<void> {
    reads.length = 0;
    h.resumeSession.mockClear();
    h.listSessionTurns.mockClear();
    h.listActiveSessions.mockResolvedValueOnce({ status: "ok", data: live.map((id) => ({ session: { id } })) });
    await reattachSessions();
  }

  beforeEach(() => {
    h.connectionStatus.mockResolvedValue({ status: "ok", data: { state: "connected", transcriptSync: true } });
    h.listSessionTurns.mockImplementation(async (id: string, cursor: LogCursorDto, limit: number) =>
      listTurns(id, cursor, limit),
    );
    h.resumeSession.mockImplementation(async (...args: unknown[]) => {
      streams.set(args[0] as string, args[args.length - 1] as InstanceType<typeof h.FakeChannel>);
      return { status: "ok", data: h.RESUMED };
    });
    useTranscriptSync.setState({ self: SELF });
  });

  afterEach(() => {
    h.connectionStatus.mockResolvedValue({ status: "ok", data: { state: "connected", transcriptSync: false } });
    h.resumeSession.mockImplementation(async () => ({ status: "ok", data: h.RESUMED }));
    h.listActiveSessions.mockResolvedValue({ status: "ok", data: [] });
    useTranscriptSync.setState({ self: null });
  });

  it("re-reads no rows for live or exited sessions when nothing changed while disconnected", async () => {
    const live = await restore(history(600), true);
    const exited = await restore(history(600), false);

    await reconnect([live]);

    expect(reads).toHaveLength(2);
    expect(reads).toEqual(
      expect.arrayContaining([
        { sessionId: live, cursor: tail(live), rows: 0 },
        { sessionId: exited, cursor: tail(exited), rows: 0 },
      ]),
    );
    expect(resumedFrom()).toEqual([[live, tail(live)]]);
    expect(texts(live)).toHaveLength(599);
    expect(texts(exited)).toHaveLength(599);
  });

  it("reads only the rows after its last read, and shows each once", async () => {
    const rows = history(600);
    const live = await restore(rows, true);
    const exited = await restore(history(600), false);
    // Applied live before the drop by a server before 0.8.21: it moves appliedSeq but carries no cursor.
    for (const seq of [600, 601]) {
      rows.push(at(seq, reply(`live ${seq}`)));
      streams.get(live)!.onmessage?.({ seq, rev: seq + 1, author: SELF, event: reply(`live ${seq}`) });
    }
    rows.push(at(602, reply("missed")), at(603, TURN_DONE));
    server.get(exited)!.rows.push(at(600, reply("missed")), at(601, TURN_DONE));

    await reconnect([live]);

    expect(reads).toEqual(
      expect.arrayContaining([
        { sessionId: live, cursor: { kind: "after", seq: 599, createdAt: rows[599].createdAt }, rows: 4 },
        { sessionId: exited, cursor: { kind: "after", seq: 599, createdAt: rows[599].createdAt }, rows: 2 },
      ]),
    );
    expect(resumedFrom()).toEqual([[live, tail(live)]]);
    expect(texts(live).slice(-3)).toEqual(["live 600", "live 601", "missed"]);
    expect(texts(exited).slice(-2)).toEqual(["r599", "missed"]);
  });

  it("reads the whole log again when a rewind moved it while disconnected", async () => {
    const moved = await restore(history(600), false);
    const rewritten = await restore(history(600), false);
    server.get(moved)!.resetRev = 7;
    server.set(rewritten, { rows: [...history(300), at(300, reply("after the rewind"), 1_800_000_000_000)], resetRev: 7 });

    await reconnect([]);

    expect(reads.filter((r) => r.cursor.kind === "empty").map((r) => r.sessionId).sort()).toEqual([moved, rewritten]);
    expect(texts(rewritten).slice(-2)).toEqual(["r299", "after the rewind"]);
    expect(syncOf(moved)?.resetRev).toBe(7);
    expect(syncOf(rewritten)?.resetRev).toBe(7);
  });

  it("does not double a live turn's streamed text when the server seeds it again", async () => {
    const live = await restore(history(10), true);
    streams.get(live)!.onmessage?.({ seq: null, rev: 10, author: SELF, event: partial("Hel") });

    await reconnect([live]);
    streams.get(live)!.onmessage?.({ seq: null, rev: 10, author: SELF, event: partial("Hello") });

    expect(texts(live).slice(-2)).toEqual(["r9", "Hello"]);
  });

  it("reconnects sessions started in this window from their last live row", async () => {
    const live = await startStreaming(turn(300));
    const exited = await startStreaming(turn(300));
    const lastLive = cursorAt(server.get(live)!.rows[300]);
    server.get(live)!.rows.push(at(301, reply("missed")), at(302, TURN_DONE));

    await reconnect([live]);

    expect(reads).toHaveLength(2);
    expect(reads).toEqual(
      expect.arrayContaining([
        { sessionId: live, cursor: lastLive, rows: 2 },
        { sessionId: exited, cursor: tail(exited), rows: 0 },
      ]),
    );
    expect(resumedFrom()).toEqual([[live, tail(live)]]);
    expect(texts(live)).toEqual([...replies(300), "missed"]);
    expect(texts(exited)).toEqual(replies(300));
  });

  it("reads only past the last live row of a restored session that kept streaming", async () => {
    const rows = history(600);
    const live = await restore(rows, true);
    for (const seq of [600, 601]) {
      rows.push(at(seq, reply(`live ${seq}`)));
      stream(live, rows[seq]);
    }
    rows.push(at(602, reply("missed")), at(603, TURN_DONE));

    await reconnect([live]);

    expect(reads).toEqual([{ sessionId: live, cursor: cursorAt(rows[601]), rows: 2 }]);
    expect(resumedFrom()).toEqual([[live, tail(live)]]);
    expect(texts(live)).toEqual([...replies(600), "live 600", "live 601", "missed"]);
  });

  it("reads the whole log again when a rewind moved its live rows while disconnected", async () => {
    const cut = await startStreaming(turn(300));
    const moved = await startStreaming(turn(300));
    const lastLive = cursorAt(server.get(cut)!.rows[300]);
    // Cut below the last live row, so that row no longer matches.
    server.set(cut, { rows: [...history(150), at(150, reply("after the rewind"), 1_800_000_000_000)], resetRev: 7 });
    // Cut only rows this window never saw: the last live row still matches, resetRev does not.
    server.set(moved, { rows: [...turn(300), at(301, reply("after the rewind"), 1_800_000_000_000)], resetRev: 7 });

    await reconnect([]);

    expect(asked()).toHaveLength(4);
    expect(asked()).toEqual(
      expect.arrayContaining([
        [cut, lastLive],
        [cut, { kind: "empty" }],
        [moved, lastLive],
        [moved, { kind: "empty" }],
      ]),
    );
    expect(texts(cut)).toEqual([...replies(150), "after the rewind"]);
    expect(texts(moved)).toEqual([...replies(300), "after the rewind"]);
    expect(syncOf(cut)?.resetRev).toBe(7);
    expect(syncOf(moved)?.resetRev).toBe(7);
  });

  it("resumes an exited session for a send after a reconnect from the held cursor", async () => {
    const exited = await restore(turn(300), false);
    await reconnect([]);
    reads.length = 0;
    h.resumeSession.mockClear();

    await sendMessage(exited, "after the reconnect");

    expect(reads).toEqual([{ sessionId: exited, cursor: tail(exited), rows: 0 }]);
    expect(resumedFrom()).toEqual([[exited, tail(exited)]]);
    expect(h.sendUserMessage).toHaveBeenCalledTimes(1);
    expect(texts(exited)).toEqual(replies(300));
  });

  it("reads the whole log for a send when a rewind moved it after the reconnect", async () => {
    const exited = await restore(turn(300), false);
    await reconnect([]);
    // The held row still matches; only resetRev says the log was rewritten.
    server.set(exited, { rows: [...turn(300), at(301, reply("after the rewind"), 1_800_000_000_000)], resetRev: 7 });
    h.listSessionTurns.mockClear();

    await sendMessage(exited, "after the rewind");

    expect(asked()).toContainEqual([exited, { kind: "empty" }]);
    expect(texts(exited)).toEqual([...replies(300), "after the rewind"]);
    expect(syncOf(exited)?.resetRev).toBe(7);
  });

  it("keeps the cursor before another device's held turn, so a reconnect reads that turn once", async () => {
    const OTHER = { clientId: "them", device: "jons-macbook-pro" };
    const id = await startStreaming(turn(10));
    const theirs = [
      at(11, { kind: "userMessage", text: "theirs", stage: null, author: OTHER }),
      at(12, reply("theirs done")),
      at(13, TURN_DONE),
    ];
    server.get(id)!.rows.push(...theirs);
    for (const row of theirs) stream(id, row, OTHER);
    expect(texts(id)).toEqual(replies(10));

    await reconnect([id]);

    expect(reads).toEqual([{ sessionId: id, cursor: cursorAt(server.get(id)!.rows[10]), rows: 3 }]);
    expect(resumedFrom()).toEqual([[id, tail(id)]]);
    expect(texts(id)).toEqual([...replies(10), "theirs done"]);
  });
});

describe("goals in the session store", () => {
  const brief = (over: Partial<GoalBrief> = {}): GoalBrief => ({
    id: "g1",
    status: "running",
    step: "working",
    pauseReason: null,
    iteration: 1,
    maxIterations: 10,
    ...over,
  });

  async function restingSession(): Promise<string> {
    const id = await startBusySession();
    emit(INIT);
    emit(TURN_DONE);
    return id;
  }

  it("activity with goal briefs", async () => {
    const id = await restingSession();
    expect(classifyEntry(entryOf(id))).toBe("review");

    setSessionGoal(id, brief({ step: "checking" }));
    expect(classifyEntry(entryOf(id))).toBe("working");
    setSessionGoal(id, brief({ step: "waiting" }));
    expect(classifyEntry(entryOf(id))).toBe("needs-you");
    setSessionGoal(id, brief({ status: "paused", step: null, pauseReason: "needs_human" }));
    expect(classifyEntry(entryOf(id))).toBe("needs-you");
    setSessionGoal(id, brief({ status: "paused", step: null, pauseReason: "user" }));
    expect(classifyEntry(entryOf(id))).toBe("review");

    emit({ kind: "userMessage", text: "go on", stage: null });
    expect(classifyEntry(entryOf(id))).toBe("working");
    setSessionGoal(id, brief({ status: "met", step: null }));
    expect(classifyEntry(entryOf(id))).toBe("review");
  });

  it("a running goal's asks don't raise a desktop notice", async () => {
    const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
    h.notifyNeedsAttention.mockClear();
    const id = await startBusySession();
    emit(INIT);
    setSessionGoal(id, brief());
    emit({ kind: "permissionAsk", requestId: "r1", toolName: "Bash", input: "{}" });
    setSessionGoal(id, brief({ step: "waiting" }));
    emit({ kind: "questionAsk", requestId: "q1", questions: [] });
    await settle();

    expect(h.notifyNeedsAttention).not.toHaveBeenCalled();
  });

  async function crashedGoalSession(): Promise<string> {
    const id = await restingSession();
    setSessionGoal(id, brief());
    emit({ kind: "exited", outcome: "crashed", code: 1, stderrTail: "" });
    h.resumeSession.mockClear();
    return id;
  }

  const reportLive = (id: string) =>
    h.listActiveSessions.mockResolvedValue({ status: "ok", data: [{ session: { id } }] });

  it("a goal running on a relaunched harness reattaches its ended stream once", async () => {
    const id = await crashedGoalSession();
    reportLive(id);

    await Promise.all([
      reattachRunningGoal(id, brief({ iteration: 2 })),
      reattachRunningGoal(id, brief({ iteration: 2, step: "checking" })),
    ]);
    expect(h.resumeSession).toHaveBeenCalledTimes(1);
    h.listActiveSessions.mockReset();
    h.listActiveSessions.mockResolvedValue({ status: "ok", data: [] });
  });

  it("a paused, ended or unchanged goal, or a dead harness, reattaches nothing", async () => {
    const id = await crashedGoalSession();
    reportLive(id);
    await reattachRunningGoal(id, brief({ status: "paused", step: null, pauseReason: "user" }));
    await reattachRunningGoal(id, brief({ status: "met", step: null }));
    await reattachRunningGoal(id, brief({ step: "resuming" }));
    await reattachRunningGoal(id, brief());
    h.listActiveSessions.mockResolvedValue({ status: "ok", data: [] });
    await reattachRunningGoal(id, brief({ iteration: 2 }));

    expect(h.resumeSession).not.toHaveBeenCalled();
  });

  it("an armed resume fires once", () => {
    expect(takeArmedResume("s-armed")).toBeNull();
    armResumeAfterSend("s-armed", "g1");
    expect(takeArmedResume("s-armed")).toBe("g1");
    expect(takeArmedResume("s-armed")).toBeNull();
  });
});
