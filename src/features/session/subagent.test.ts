import { describe, expect, it } from "vitest";

import { runningSubagent } from "./subagent";
import type { TranscriptItem } from "./transcript";

type Tool = Extract<TranscriptItem, { kind: "tool" }>;

const ok = (content: string) => ({ content, isError: false });

function tool(
  name: string,
  input: Record<string, unknown>,
  toolUseId: string,
  opts: { parentId?: string; result?: Tool["result"] } = {},
): Tool {
  return {
    kind: "tool",
    id: `id-${toolUseId}`,
    toolUseId,
    name,
    input: JSON.stringify(input),
    result: opts.result ?? null,
    parentId: opts.parentId ?? null,
  };
}

const foreman = (id: string, result: Tool["result"] = null) =>
  tool(
    "Agent",
    { description: "Foreman: implement plan Phase 1", subagent_type: "foreman", prompt: "…", run_in_background: false },
    id,
    { result },
  );
const read = (id: string, parentId: string, file: string) =>
  tool("Read", { file_path: file }, id, { parentId, result: ok("…") });
const edit = (id: string, parentId: string, file: string) =>
  tool("Edit", { file_path: file, old_string: "a", new_string: "b" }, id, { parentId, result: ok("ok") });
const todos = (id: string, parentId: string, list: [string, string][]) =>
  tool("TodoWrite", { todos: list.map(([content, status]) => ({ content, status, activeForm: content })) }, id, {
    parentId,
    result: ok("ok"),
  });

describe("runningSubagent", () => {
  it("is null without a subagent", () => {
    expect(runningSubagent([tool("Read", { file_path: "/repo/a.rs" }, "r1", { result: ok("…") })])).toBeNull();
  });

  it("reports a foreground subagent until its result arrives", () => {
    const children = [read("r1", "f1", "/repo/a.rs"), edit("e1", "f1", "/repo/src/review/diff.rs")];
    expect(runningSubagent([foreman("f1"), ...children])).toEqual({
      toolUseId: "f1",
      agent: "foreman",
      background: false,
      tools: 2,
      lastTool: "Edit diff.rs",
      steps: null,
      phase: "1",
    });
    expect(runningSubagent([foreman("f1", ok("done")), ...children])).toBeNull();
  });

  it("counts only its own tool calls, not a nested subagent's", () => {
    const nested: TranscriptItem[] = [
      { kind: "assistant", id: "a1", text: "Looking around", streaming: false, parentId: "f1" },
      tool("Agent", { subagent_type: "codebase-locator", run_in_background: false }, "n1", { parentId: "f1" }),
      read("r1", "n1", "/repo/b.rs"),
      read("r2", "n1", "/repo/c.rs"),
    ];
    expect(runningSubagent([foreman("f1"), ...nested])).toMatchObject({
      toolUseId: "f1",
      tools: 1,
      lastTool: "Agent codebase-locator",
    });
    expect(runningSubagent([foreman("f1", ok("done")), ...nested])).toBeNull();
  });

  it("reads steps from its latest todo list", () => {
    const items = [
      foreman("f1"),
      todos("w1", "f1", [
        ["Write the E2E scenario", "in_progress"],
        ["Diff layer", "pending"],
        ["IPC", "pending"],
      ]),
      read("r1", "f1", "/repo/a.rs"),
      todos("w2", "f1", [
        ["Write the E2E scenario", "completed"],
        ["Diff layer", "in_progress"],
        ["IPC", "pending"],
      ]),
      edit("e1", "f1", "/repo/diff.rs"),
    ];
    expect(runningSubagent(items)?.steps).toEqual({ done: 1, total: 3, current: "Diff layer" });
  });

  it("keeps a background subagent running after its spawn result until its task completes", () => {
    const items: TranscriptItem[] = [
      tool("Agent", { description: "Map the store", subagent_type: "cartographer" }, "b1", {
        result: ok("Async agent launched successfully."),
      }),
      read("r1", "b1", "/repo/a.rs"),
    ];
    expect(runningSubagent(items)).toMatchObject({ toolUseId: "b1", agent: "cartographer", background: true, tools: 1 });
    const done: TranscriptItem = { kind: "taskUpdate", id: "t1", taskId: "task-b1", toolUseId: "b1", status: "completed" };
    expect(runningSubagent([...items, done])).toBeNull();
  });

  it("picks the most recently started of parallel subagents", () => {
    const items = [
      foreman("f1"),
      foreman("f2"),
      read("r1", "f1", "/repo/a.rs"),
      edit("e1", "f2", "/repo/b.rs"),
      read("r2", "f1", "/repo/c.rs"),
    ];
    expect(runningSubagent(items)).toMatchObject({ toolUseId: "f2", tools: 1, lastTool: "Edit b.rs" });
  });

  it("forgets a subagent once the harness exits or relaunches", () => {
    const items = [foreman("f1"), read("r1", "f1", "/repo/a.rs")];
    expect(runningSubagent([...items, { kind: "relaunched", id: "x1" }])).toBeNull();
    expect(
      runningSubagent([...items, { kind: "exit", id: "x2", outcome: "crashed", code: 1, stderrTail: "" }]),
    ).toBeNull();
  });
});
