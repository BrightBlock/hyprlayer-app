import { describe, expect, it } from "vitest";

import type { SessionEvent } from "@/lib/bindings";
import {
  backgroundTasksFromItems,
  isBackgroundSpawn,
  type BackgroundTask,
} from "./backgroundTask";
import { initialSessionView, reduceEvent, type TranscriptItem } from "./transcript";
import fixture from "./__fixtures__/background-tasks.events.json";

// ─── helpers ────────────────────────────────────────────────────────────────

const SPAWN_RESULT = (taskId: string) => ({
  content: `Command running in background with ID: ${taskId}. Output is being written to: /tmp/tasks/${taskId}.output. You will be notified when it completes. To check interim output, use Read on that file path.`,
  isError: false,
});

const TASKOUT_TIMEOUT = (taskId: string) => ({
  content: `<retrieval_status>timeout</retrieval_status>\n\n<task_id>${taskId}</task_id>\n\n<task_type>local_bash</task_type>\n\n<status>running</status>`,
  isError: false,
});

const TASKOUT_COMPLETED = (taskId: string) => ({
  content: `<retrieval_status>success</retrieval_status>\n\n<task_id>${taskId}</task_id>\n\n<task_type>local_bash</task_type>\n\n<status>completed</status>\n\n<exit_code>0</exit_code>\n\n<output>\nDone.\n</output>`,
  isError: false,
});

const TASKSTOP_RESULT = (taskId: string, command = "some-cmd") => ({
  content: JSON.stringify({
    message: `Successfully stopped task: ${taskId} (${command})`,
    task_id: taskId,
    task_type: "local_bash",
    command,
  }),
  isError: false,
});

function makeTool(
  name: string,
  input: string,
  toolUseId: string,
  result: { content: string; isError: boolean } | null = null,
): Extract<TranscriptItem, { kind: "tool" }> {
  return { kind: "tool", id: `id-${toolUseId}`, toolUseId, name, input, result };
}

function bgBash(
  toolUseId: string,
  opts: {
    command?: string;
    description?: string;
    result?: { content: string; isError: boolean } | null;
  } = {},
): Extract<TranscriptItem, { kind: "tool" }> {
  const input: Record<string, unknown> = {
    command: opts.command ?? "sleep 999",
    run_in_background: true,
  };
  if (opts.description !== undefined) input.description = opts.description;
  return makeTool("Bash", JSON.stringify(input), toolUseId, opts.result ?? null);
}

function fgBash(toolUseId: string, command = "ls"): Extract<TranscriptItem, { kind: "tool" }> {
  return makeTool("Bash", JSON.stringify({ command }), toolUseId, { content: "done", isError: false });
}

function taskOutput(
  toolUseId: string,
  taskId: string,
  result: { content: string; isError: boolean } | null,
  block = true,
): Extract<TranscriptItem, { kind: "tool" }> {
  return makeTool(
    "TaskOutput",
    JSON.stringify({ task_id: taskId, block, timeout: 60000 }),
    toolUseId,
    result,
  );
}

function taskStop(
  toolUseId: string,
  taskId: string,
  result: { content: string; isError: boolean } | null = null,
): Extract<TranscriptItem, { kind: "tool" }> {
  return makeTool("TaskStop", JSON.stringify({ task_id: taskId }), toolUseId, result);
}

/** A background-task notification marker (the reduced `backgroundTaskUpdate`
 *  event). `spawnToolUseId` mirrors the notification carrying the spawning
 *  Bash tool-use id directly; pass null to exercise the task_id fallback. */
function taskUpdate(
  taskId: string,
  spawnToolUseId: string | null,
  status: string,
): Extract<TranscriptItem, { kind: "taskUpdate" }> {
  return { kind: "taskUpdate", id: `up-${taskId}-${status}`, taskId, toolUseId: spawnToolUseId, status };
}

/** A process-boundary marker (backend relaunch). Clears tracked tasks. */
function relaunchedMarker(id = "relaunch-1"): Extract<TranscriptItem, { kind: "relaunched" }> {
  return { kind: "relaunched", id };
}

/** A live-rewind boundary (SAME process). Must NOT clear tracked tasks. */
function rewoundMarker(id = "rewound-1"): Extract<TranscriptItem, { kind: "rewound" }> {
  return { kind: "rewound", id };
}

function exitItem(id = "exit-1"): Extract<TranscriptItem, { kind: "exit" }> {
  return { kind: "exit", id, outcome: "killed", code: 143, stderrTail: "" };
}

function replayFixture(events: unknown[]): TranscriptItem[] {
  let view = initialSessionView();
  for (const ev of events) {
    view = reduceEvent(view, ev as SessionEvent);
  }
  return view.items;
}

// ─── isBackgroundSpawn ───────────────────────────────────────────────────────

describe("isBackgroundSpawn", () => {
  it("returns true for a Bash item with run_in_background:true", () => {
    expect(isBackgroundSpawn(bgBash("t1"))).toBe(true);
  });

  it("returns false for a regular foreground Bash item", () => {
    expect(isBackgroundSpawn(fgBash("t1"))).toBe(false);
  });

  it("returns false for non-Bash tool items", () => {
    expect(
      isBackgroundSpawn(makeTool("TaskOutput", JSON.stringify({ task_id: "x" }), "t1")),
    ).toBe(false);
  });

  it("returns false for non-tool transcript items", () => {
    const userItem: TranscriptItem = {
      kind: "user",
      id: "u1",
      text: "hello",
      stage: null,
      attachments: [],
      createdAt: null, promptName: null
    };
    expect(isBackgroundSpawn(userItem)).toBe(false);
  });
});

// ─── backgroundTasksFromItems ────────────────────────────────────────────────

describe("backgroundTasksFromItems", () => {
  it("returns empty array for an empty transcript", () => {
    expect(backgroundTasksFromItems([])).toEqual([]);
  });

  it("ignores foreground Bash calls", () => {
    const items: TranscriptItem[] = [fgBash("t1"), fgBash("t2")];
    expect(backgroundTasksFromItems(items)).toHaveLength(0);
  });

  it("ignores non-tool transcript items", () => {
    const items: TranscriptItem[] = [
      { kind: "user", id: "u1", text: "hi", stage: null, attachments: [], createdAt: null, promptName: null },
      { kind: "assistant", id: "a1", text: "hello", streaming: false },
    ];
    expect(backgroundTasksFromItems(items)).toHaveLength(0);
  });

  it("adds a running task when spawn + result arrive", () => {
    const items: TranscriptItem[] = [bgBash("t1", { description: "My task", result: SPAWN_RESULT("task-a") })];
    const tasks = backgroundTasksFromItems(items);
    expect(tasks).toHaveLength(1);
    expect(tasks[0]).toMatchObject<Partial<BackgroundTask>>({
      taskId: "task-a",
      toolUseId: "t1",
      label: "My task",
      status: "running",
    });
  });

  it("uses command first-line as label when description is absent", () => {
    const items: TranscriptItem[] = [
      bgBash("t1", { command: "pnpm build\nsome-other-line", result: SPAWN_RESULT("task-a") }),
    ];
    const tasks = backgroundTasksFromItems(items);
    expect(tasks[0].label).toBe("pnpm build …");
  });

  it("sets taskId to null before the spawn result arrives", () => {
    // result === null means the toolResult event hasn't arrived yet
    const items: TranscriptItem[] = [bgBash("t1", { result: null })];
    const tasks = backgroundTasksFromItems(items);
    expect(tasks).toHaveLength(1);
    expect(tasks[0].taskId).toBeNull();
    expect(tasks[0].status).toBe("running");
  });

  it("flips status to awaiting when TaskOutput result is pending", () => {
    const items: TranscriptItem[] = [
      bgBash("t1", { result: SPAWN_RESULT("task-a") }),
      taskOutput("to1", "task-a", null), // result=null → agent is blocked
    ];
    const tasks = backgroundTasksFromItems(items);
    expect(tasks).toHaveLength(1);
    expect(tasks[0].status).toBe("awaiting");
    expect(tasks[0].taskId).toBe("task-a");
  });

  it("resets status to running after a TaskOutput timeout result", () => {
    const items: TranscriptItem[] = [
      bgBash("t1", { result: SPAWN_RESULT("task-a") }),
      taskOutput("to1", "task-a", null), // awaiting
      taskOutput("to2", "task-a", TASKOUT_TIMEOUT("task-a")), // timeout → still running
    ];
    const tasks = backgroundTasksFromItems(items);
    expect(tasks).toHaveLength(1);
    expect(tasks[0].status).toBe("running");
  });

  it("removes a task when TaskOutput result indicates completion", () => {
    const items: TranscriptItem[] = [
      bgBash("t1", { result: SPAWN_RESULT("task-a") }),
      taskOutput("to1", "task-a", TASKOUT_COMPLETED("task-a")),
    ];
    expect(backgroundTasksFromItems(items)).toHaveLength(0);
  });

  it("removes a task when TaskStop result arrives", () => {
    const items: TranscriptItem[] = [
      bgBash("t1", { result: SPAWN_RESULT("task-a") }),
      taskStop("ts1", "task-a", TASKSTOP_RESULT("task-a", "sleep 999")),
    ];
    expect(backgroundTasksFromItems(items)).toHaveLength(0);
  });

  // ── completion notification (the fire-and-forget path) ─────────────────────

  it("removes a task on a terminal completion notification, agent never polled", () => {
    // The screenshot bug: a `sleep`-style task the agent fires and forgets. No
    // TaskOutput poll, no TaskStop, only claude's async notification clears it.
    const items: TranscriptItem[] = [
      bgBash("t1", { result: SPAWN_RESULT("task-a") }),
      taskUpdate("task-a", "t1", "completed"),
    ];
    expect(backgroundTasksFromItems(items)).toHaveLength(0);
  });

  it("removes a task on a FAILED completion notification (no longer running)", () => {
    const items: TranscriptItem[] = [
      bgBash("t1", { result: SPAWN_RESULT("task-a") }),
      taskUpdate("task-a", "t1", "failed"),
    ];
    expect(backgroundTasksFromItems(items)).toHaveLength(0);
  });

  it("matches the notification by task_id when it carries no toolUseId", () => {
    // Defensive fallback: an update keyed only on task_id still resolves to the
    // spawning tool via the spawn-result mapping.
    const items: TranscriptItem[] = [
      bgBash("t1", { result: SPAWN_RESULT("task-a") }),
      taskUpdate("task-a", null, "completed"),
    ];
    expect(backgroundTasksFromItems(items)).toHaveLength(0);
  });

  it("keeps the task on a non-terminal 'running' notification", () => {
    const items: TranscriptItem[] = [
      bgBash("t1", { result: SPAWN_RESULT("task-a") }),
      taskUpdate("task-a", "t1", "running"),
    ];
    const tasks = backgroundTasksFromItems(items);
    expect(tasks).toHaveLength(1);
    expect(tasks[0].taskId).toBe("task-a");
  });

  it("removes only the notified task when two are running", () => {
    const items: TranscriptItem[] = [
      bgBash("t1", { description: "Task A", result: SPAWN_RESULT("task-a") }),
      bgBash("t2", { description: "Task B", result: SPAWN_RESULT("task-b") }),
      taskUpdate("task-a", "t1", "completed"),
    ];
    const tasks = backgroundTasksFromItems(items);
    expect(tasks).toHaveLength(1);
    expect(tasks[0].taskId).toBe("task-b");
  });

  it("ignores a completion notification for an unknown task", () => {
    const items: TranscriptItem[] = [
      bgBash("t1", { result: SPAWN_RESULT("task-a") }),
      taskUpdate("completely-unknown", null, "completed"),
    ];
    expect(backgroundTasksFromItems(items)).toHaveLength(1);
  });

  // ── errored spawn (denied / hook-blocked), no phantom ─────────────────────

  it("does not track a background spawn whose result is an error", () => {
    const items: TranscriptItem[] = [
      bgBash("t1", {
        description: "Blocked build",
        result: { content: "Permission to use Bash was denied.", isError: true },
      }),
    ];
    expect(backgroundTasksFromItems(items)).toHaveLength(0);
  });

  // ── process death (relaunch / exit) clears orphaned tasks ──────────────────

  it("clears tracked tasks at a relaunched boundary (resume/restart/log-only rewind)", () => {
    const items: TranscriptItem[] = [
      bgBash("t1", { description: "Windows build", result: SPAWN_RESULT("task-a") }),
      relaunchedMarker(),
    ];
    expect(backgroundTasksFromItems(items)).toHaveLength(0);
  });

  it("keeps a task spawned AFTER the relaunch boundary (the new process is alive)", () => {
    const items: TranscriptItem[] = [
      bgBash("t1", { description: "Old gen", result: SPAWN_RESULT("task-a") }),
      relaunchedMarker(),
      bgBash("t2", { description: "New gen", result: SPAWN_RESULT("task-b") }),
    ];
    const tasks = backgroundTasksFromItems(items);
    expect(tasks).toHaveLength(1);
    expect(tasks[0].taskId).toBe("task-b");
    expect(tasks[0].label).toBe("New gen");
  });

  it("KEEPS tracked tasks at a rewound boundary (live rewind, same process)", () => {
    // A live rewind peels the conversation but leaves the process (and its
    // background shells) running, so the task must survive the boundary, unlike
    // a relaunch, which kills the process.
    const items: TranscriptItem[] = [
      bgBash("t1", { description: "Long build", result: SPAWN_RESULT("task-a") }),
      rewoundMarker(),
    ];
    const tasks = backgroundTasksFromItems(items);
    expect(tasks).toHaveLength(1);
    expect(tasks[0].taskId).toBe("task-a");
  });

  it("clears tracked tasks at an exit item (the process ended)", () => {
    const items: TranscriptItem[] = [
      bgBash("t1", { result: SPAWN_RESULT("task-a") }),
      exitItem(),
    ];
    expect(backgroundTasksFromItems(items)).toHaveLength(0);
  });

  // ── output-file read (the dominant real-world terminus) ────────────────────

  it("ends a task when the agent reads its .output file via Read", () => {
    const items: TranscriptItem[] = [
      bgBash("t1", { result: SPAWN_RESULT("task-a") }),
      makeTool("Read", JSON.stringify({ file_path: "/tmp/tasks/task-a.output" }), "rd1", {
        content: "…build log…",
        isError: false,
      }),
    ];
    expect(backgroundTasksFromItems(items)).toHaveLength(0);
  });

  it("ends a task when the agent greps its .output file via foreground Bash (the screenshot bug)", () => {
    // Real repro: the agent backgrounds `cargo test` to regenerate TS bindings,
    // then greps the output file directly. claude suppresses the completion
    // notification once the output is read, so this read is the only signal that
    // ever clears the task.
    const items: TranscriptItem[] = [
      bgBash("t1", {
        command: "cargo test typescript_bindings_export",
        description: "Regenerate TS bindings",
        result: SPAWN_RESULT("task-a"),
      }),
      makeTool(
        "Bash",
        JSON.stringify({ command: 'grep -iE "warning|error" /tmp/tasks/task-a.output' }),
        "g1",
        { content: "(clean)", isError: false },
      ),
    ];
    expect(backgroundTasksFromItems(items)).toHaveLength(0);
  });

  it("drops the read task as soon as the read tool-use appears, before its result", () => {
    // The toolUse already carries the path in its input, so we clear promptly
    // rather than waiting for the read's result to arrive.
    const items: TranscriptItem[] = [
      bgBash("t1", { result: SPAWN_RESULT("task-a") }),
      makeTool("Read", JSON.stringify({ file_path: "/tmp/tasks/task-a.output" }), "rd1", null),
    ];
    expect(backgroundTasksFromItems(items)).toHaveLength(0);
  });

  it("ends only the read task when several are running", () => {
    const items: TranscriptItem[] = [
      bgBash("t1", { description: "Task A", result: SPAWN_RESULT("task-a") }),
      bgBash("t2", { description: "Task B", result: SPAWN_RESULT("task-b") }),
      makeTool("Read", JSON.stringify({ file_path: "/tmp/tasks/task-b.output" }), "rd1", null),
    ];
    const tasks = backgroundTasksFromItems(items);
    expect(tasks).toHaveLength(1);
    expect(tasks[0].taskId).toBe("task-a");
  });

  it("leaves tasks intact for a tool that references no .output path", () => {
    const items: TranscriptItem[] = [
      bgBash("t1", { result: SPAWN_RESULT("task-a") }),
      fgBash("fg1", "ls -la"),
      makeTool("Read", JSON.stringify({ file_path: "/some/other/file.txt" }), "rd1", null),
    ];
    const tasks = backgroundTasksFromItems(items);
    expect(tasks).toHaveLength(1);
    expect(tasks[0].taskId).toBe("task-a");
  });

  it("does NOT remove a task when TaskStop has no result yet (not confirmed)", () => {
    const items: TranscriptItem[] = [
      bgBash("t1", { result: SPAWN_RESULT("task-a") }),
      taskStop("ts1", "task-a", null), // result=null → stop not confirmed
    ];
    const tasks = backgroundTasksFromItems(items);
    expect(tasks).toHaveLength(1);
  });

  it("supports legacy KillBash (shell_id) for stopping tasks", () => {
    const items: TranscriptItem[] = [
      bgBash("t1", { result: SPAWN_RESULT("task-a") }),
      makeTool(
        "KillBash",
        JSON.stringify({ shell_id: "task-a" }),
        "kb1",
        { content: "killed", isError: false },
      ),
    ];
    expect(backgroundTasksFromItems(items)).toHaveLength(0);
  });

  it("supports legacy BashOutput (shell_id) for awaiting", () => {
    const items: TranscriptItem[] = [
      bgBash("t1", { result: SPAWN_RESULT("task-a") }),
      makeTool("BashOutput", JSON.stringify({ shell_id: "task-a" }), "bo1", null),
    ];
    const tasks = backgroundTasksFromItems(items);
    expect(tasks).toHaveLength(1);
    expect(tasks[0].status).toBe("awaiting");
  });

  it("tracks two concurrent background tasks independently", () => {
    const items: TranscriptItem[] = [
      bgBash("t1", { description: "Task A", result: SPAWN_RESULT("task-a") }),
      bgBash("t2", { description: "Task B", result: SPAWN_RESULT("task-b") }),
      taskOutput("to1", "task-a", null), // A is awaiting
    ];
    const tasks = backgroundTasksFromItems(items);
    expect(tasks).toHaveLength(2);
    const a = tasks.find((t) => t.taskId === "task-a")!;
    const b = tasks.find((t) => t.taskId === "task-b")!;
    expect(a.status).toBe("awaiting");
    expect(b.status).toBe("running");
  });

  it("removes only the stopped task when two are running", () => {
    const items: TranscriptItem[] = [
      bgBash("t1", { description: "Task A", result: SPAWN_RESULT("task-a") }),
      bgBash("t2", { description: "Task B", result: SPAWN_RESULT("task-b") }),
      taskStop("ts1", "task-a", TASKSTOP_RESULT("task-a")),
    ];
    const tasks = backgroundTasksFromItems(items);
    expect(tasks).toHaveLength(1);
    expect(tasks[0].taskId).toBe("task-b");
  });

  it("preserves spawn order", () => {
    const items: TranscriptItem[] = [
      bgBash("t1", { description: "First", result: SPAWN_RESULT("id-1") }),
      bgBash("t2", { description: "Second", result: SPAWN_RESULT("id-2") }),
      bgBash("t3", { description: "Third", result: SPAWN_RESULT("id-3") }),
    ];
    const tasks = backgroundTasksFromItems(items);
    expect(tasks.map((t) => t.label)).toEqual(["First", "Second", "Third"]);
  });

  it("ignores TaskOutput for an unknown task_id", () => {
    const items: TranscriptItem[] = [
      bgBash("t1", { result: SPAWN_RESULT("task-a") }),
      taskOutput("to1", "completely-unknown", null), // unknown id → no effect
    ];
    const tasks = backgroundTasksFromItems(items);
    expect(tasks).toHaveLength(1);
    expect(tasks[0].status).toBe("running");
  });
});

// ─── fixture replay ──────────────────────────────────────────────────────────

describe("backgroundTasksFromItems, fixture replay", () => {
  it("ends up with zero live tasks after the full fixture sequence", () => {
    const items = replayFixture(fixture);
    // The fixture goes: spawn → poll-timeout → poll-completed → turnDone
    // After all events are replayed, the task is completed and removed.
    expect(backgroundTasksFromItems(items)).toHaveLength(0);
  });

  it("pins the Bash-birth wire format: extracts task_id from spawn result", () => {
    // Replay only the first two events (toolUse + toolResult for spawn)
    const items = replayFixture(fixture.slice(0, 2));
    const tasks = backgroundTasksFromItems(items);
    expect(tasks).toHaveLength(1);
    expect(tasks[0].taskId).toBe("fixture-task-1");
    expect(tasks[0].label).toBe("Launch dev instance");
    expect(tasks[0].status).toBe("running");
  });

  it("shows awaiting while TaskOutput result is pending (awaiting state)", () => {
    // Replay spawn + result + turnDone + TaskOutput (NO toolResult yet)
    const items = replayFixture(fixture.slice(0, 4));
    const tasks = backgroundTasksFromItems(items);
    expect(tasks).toHaveLength(1);
    expect(tasks[0].status).toBe("awaiting");
  });

  it("resets to running after a timeout result", () => {
    // Replay through the timeout result (events 0-4)
    const items = replayFixture(fixture.slice(0, 5));
    const tasks = backgroundTasksFromItems(items);
    expect(tasks).toHaveLength(1);
    expect(tasks[0].status).toBe("running");
  });

  it("a reduced backgroundTaskUpdate event drops a fire-and-forget task", () => {
    // End-to-end: spawn a background task, end the turn (no poll), then deliver
    // claude's async completion notification as a real event through the reducer.
    const events: SessionEvent[] = [
      { kind: "toolUse", id: "bg-1", name: "Bash", input: JSON.stringify({ command: "sleep 150", description: "Sleep 2.5 minutes", run_in_background: true }), parentId: null },
      { kind: "toolResult", toolUseId: "bg-1", content: SPAWN_RESULT("task-z").content, isError: false, parentId: null },
      { kind: "turnDone", costUsd: null, numTurns: 1, contextTokens: null },
    ];
    const before = replayFixture(events);
    expect(backgroundTasksFromItems(before)).toHaveLength(1);

    const after = replayFixture([
      ...events,
      { kind: "backgroundTaskUpdate", taskId: "task-z", toolUseId: "bg-1", status: "completed" },
    ]);
    expect(backgroundTasksFromItems(after)).toHaveLength(0);
  });

  it("a direct output-file read clears a fire-and-forget task end-to-end", () => {
    // The real-world path: background a build, end the turn, then in a later
    // turn read the output file directly (claude sends no completion
    // notification once the output is consumed, so the read must clear it).
    const events: SessionEvent[] = [
      { kind: "toolUse", id: "bg-1", name: "Bash", input: JSON.stringify({ command: "cargo test", description: "Regenerate TS bindings", run_in_background: true }), parentId: null },
      { kind: "toolResult", toolUseId: "bg-1", content: SPAWN_RESULT("task-z").content, isError: false, parentId: null },
      { kind: "turnDone", costUsd: null, numTurns: 1, contextTokens: null },
    ];
    expect(backgroundTasksFromItems(replayFixture(events))).toHaveLength(1);

    const afterRead = replayFixture([
      ...events,
      { kind: "toolUse", id: "rd-1", name: "Bash", input: JSON.stringify({ command: "tail -25 /private/tmp/tasks/task-z.output" }), parentId: null },
    ]);
    expect(backgroundTasksFromItems(afterRead)).toHaveLength(0);
  });

  it("keeps a fire-and-forget task alive across a later turn's init (init is per-turn, not a restart)", () => {
    // Regression guard: `init` is per-turn (prod: 19 inits / 3 exits), so a task
    // must SURVIVE a later turn's init. Clearing on init wrongly dropped it.
    const before = replayFixture([
      { kind: "init", claudeSessionId: "sess-1", model: "claude-haiku-4-5-20251001" },
      { kind: "toolUse", id: "bg-1", name: "Bash", input: JSON.stringify({ command: "sleep 600", description: "Long build", run_in_background: true }), parentId: null },
      { kind: "toolResult", toolUseId: "bg-1", content: SPAWN_RESULT("task-z").content, isError: false, parentId: null },
      { kind: "turnDone", costUsd: null, numTurns: 1, contextTokens: null },
    ]);
    expect(backgroundTasksFromItems(before)).toHaveLength(1);

    const afterNextTurn = replayFixture([
      { kind: "init", claudeSessionId: "sess-1", model: "claude-haiku-4-5-20251001" },
      { kind: "toolUse", id: "bg-1", name: "Bash", input: JSON.stringify({ command: "sleep 600", description: "Long build", run_in_background: true }), parentId: null },
      { kind: "toolResult", toolUseId: "bg-1", content: SPAWN_RESULT("task-z").content, isError: false, parentId: null },
      { kind: "turnDone", costUsd: null, numTurns: 1, contextTokens: null },
      { kind: "userMessage", text: "reply ok", stage: null },
      { kind: "init", claudeSessionId: "sess-1", model: "claude-haiku-4-5-20251001" },
      { kind: "assistantText", text: "ok", parentId: null },
      { kind: "turnDone", costUsd: null, numTurns: 1, contextTokens: null },
    ]);
    const live = backgroundTasksFromItems(afterNextTurn);
    expect(live).toHaveLength(1);
    expect(live[0].taskId).toBe("task-z");
  });

  it("a relaunched event clears a fire-and-forget task end-to-end", () => {
    const events: SessionEvent[] = [
      { kind: "init", claudeSessionId: "s1", model: "claude-opus-4-8" },
      { kind: "toolUse", id: "bg-1", name: "Bash", input: JSON.stringify({ command: "ssh nexus-win 'cargo build --release'", description: "Windows build", run_in_background: true }), parentId: null },
      { kind: "toolResult", toolUseId: "bg-1", content: SPAWN_RESULT("task-z").content, isError: false, parentId: null },
      { kind: "turnDone", costUsd: null, numTurns: 1, contextTokens: null },
    ];
    expect(backgroundTasksFromItems(replayFixture(events))).toHaveLength(1);

    const afterRelaunch = replayFixture([...events, { kind: "relaunched" }]);
    expect(backgroundTasksFromItems(afterRelaunch)).toHaveLength(0);
  });
});
