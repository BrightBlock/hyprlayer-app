import { describe, expect, it } from "vitest";

import type { SessionEvent } from "../../lib/bindings";
import {
  addUserMessage,
  describeState,
  initialSessionView,
  isBusy,
  isTerminal,
  reduceEvent,
  resolvePermission,
  viewToText,
  type SessionView,
  type TranscriptItem,
} from "./transcript";

function reduceAll(events: SessionEvent[], start = initialSessionView("starting")): SessionView {
  return events.reduce(reduceEvent, start);
}

function kinds(view: SessionView): TranscriptItem["kind"][] {
  return view.items.map((i) => i.kind);
}

describe("text coalescing", () => {
  it("accumulates partials into one streaming block, then the final replaces it", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "partialText", text: "Hel" });
    view = reduceEvent(view, { kind: "partialText", text: "lo wor" });
    expect(view.items).toHaveLength(1);
    expect(view.items[0]).toMatchObject({ kind: "assistant", text: "Hello wor", streaming: true });

    view = reduceEvent(view, { kind: "assistantText", text: "Hello world." });
    expect(view.items).toHaveLength(1);
    expect(view.items[0]).toMatchObject({ kind: "assistant", text: "Hello world.", streaming: false });
  });

  it("creates a finalized block when assistantText arrives with no partials", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "assistantText", text: "Direct." });
    expect(view.items).toHaveLength(1);
    expect(view.items[0]).toMatchObject({ kind: "assistant", text: "Direct.", streaming: false });
  });

  it("keeps accumulated text when the final block is empty", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "partialText", text: "streamed" });
    view = reduceEvent(view, { kind: "assistantText", text: "" });
    expect(view.items[0]).toMatchObject({ text: "streamed", streaming: false });
  });

  it("ignores empty finals and empty partials with nothing open", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "assistantText", text: "" });
    view = reduceEvent(view, { kind: "partialText", text: "" });
    expect(view.items).toHaveLength(0);
  });

  it("starts a fresh block after a tool interrupts the text", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "partialText", text: "first" });
    view = reduceEvent(view, { kind: "assistantText", text: "first" });
    view = reduceEvent(view, { kind: "toolUse", id: "t1", name: "Read", input: "{}" });
    view = reduceEvent(view, { kind: "partialText", text: "second" });
    expect(kinds(view)).toEqual(["assistant", "tool", "assistant"]);
    expect(view.items[2]).toMatchObject({ text: "second", streaming: true });
  });
});

describe("tool blocks", () => {
  it("pairs a tool_result with its tool_use by id", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "toolUse", id: "abc", name: "Bash", input: '{"command":"ls"}' });
    expect(view.items[0]).toMatchObject({ kind: "tool", toolUseId: "abc", result: null });

    view = reduceEvent(view, { kind: "toolResult", toolUseId: "abc", content: "file.txt", isError: false });
    expect(view.items).toHaveLength(1);
    expect(view.items[0]).toMatchObject({
      kind: "tool",
      result: { content: "file.txt", isError: false },
    });
  });

  it("surfaces an orphan tool_result as its own block", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "toolResult", toolUseId: "ghost", content: "x", isError: true });
    expect(view.items).toHaveLength(1);
    expect(view.items[0]).toMatchObject({ kind: "tool", name: "result", result: { isError: true } });
  });

  it("matches the right tool when several are open", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "toolUse", id: "t1", name: "Read", input: "{}" });
    view = reduceEvent(view, { kind: "toolUse", id: "t2", name: "Grep", input: "{}" });
    view = reduceEvent(view, { kind: "toolResult", toolUseId: "t1", content: "r1", isError: false });
    const t1 = view.items.find((i) => i.kind === "tool" && i.toolUseId === "t1");
    const t2 = view.items.find((i) => i.kind === "tool" && i.toolUseId === "t2");
    expect(t1).toMatchObject({ result: { content: "r1" } });
    expect(t2).toMatchObject({ result: null });
  });
});

describe("thinking", () => {
  it("adds a thinking block and closes any open assistant text", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "partialText", text: "hmm" });
    view = reduceEvent(view, { kind: "thinking", text: "reasoning" });
    expect(kinds(view)).toEqual(["assistant", "thinking"]);
    expect(view.items[0]).toMatchObject({ kind: "assistant", streaming: false });
  });
});

describe("permission flow", () => {
  it("parks the session and records the pending ask", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, {
      kind: "permissionAsk",
      requestId: "req-1",
      toolName: "Bash",
      input: '{"command":"git push"}',
    });
    expect(view.state).toBe("awaiting_permission");
    expect(view.pendingPermission).toEqual({
      requestId: "req-1",
      toolName: "Bash",
      input: '{"command":"git push"}',
    });
    expect(view.items[0]).toMatchObject({ kind: "permission", decision: null });
  });

  it("resolvePermission marks the card, clears pending, and resumes", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, {
      kind: "permissionAsk",
      requestId: "req-1",
      toolName: "Bash",
      input: "{}",
    });
    view = resolvePermission(view, "req-1", "allow");
    expect(view.pendingPermission).toBeNull();
    expect(view.state).toBe("running");
    expect(view.streaming).toBe(true);
    expect(view.items[0]).toMatchObject({ kind: "permission", decision: "allow" });
  });

  it("a stale resolve does not clear an unrelated pending ask", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "permissionAsk", requestId: "req-2", toolName: "Edit", input: "{}" });
    view = resolvePermission(view, "req-1", "deny");
    expect(view.pendingPermission?.requestId).toBe("req-2");
  });
});

describe("turn + exit lifecycle", () => {
  it("turnDone closes streaming and records cumulative cost/turns", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "partialText", text: "answer" });
    view = reduceEvent(view, { kind: "turnDone", costUsd: 0.0123, numTurns: 2 });
    expect(view.streaming).toBe(false);
    expect(view.state).toBe("running");
    expect(view.costUsd).toBeCloseTo(0.0123);
    expect(view.numTurns).toBe(2);
    expect(view.items[0]).toMatchObject({ kind: "assistant", streaming: false });
  });

  it("exited appends a terminal block and sets the outcome state", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, {
      kind: "exited",
      outcome: "crashed",
      code: 1,
      stderrTail: "boom",
    });
    expect(view.state).toBe("crashed");
    expect(view.streaming).toBe(false);
    expect(view.items[view.items.length - 1]).toMatchObject({
      kind: "exit",
      outcome: "crashed",
      stderrTail: "boom",
    });
    expect(isTerminal(view.state)).toBe(true);
  });

  it("protocolError surfaces without disrupting state", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "protocolError", detail: "bad line", raw: "{garbage" });
    expect(view.items[0]).toMatchObject({ kind: "error", detail: "bad line" });
    expect(view.state).toBe("running");
  });
});

describe("replay determinism", () => {
  it("rebuilds the same transcript from a recorded stream", () => {
    const events: SessionEvent[] = [
      { kind: "init", claudeSessionId: "sess-x", model: "claude-opus-4-8" },
      { kind: "partialText", text: "Look" },
      { kind: "partialText", text: "ing…" },
      { kind: "assistantText", text: "Looking…" },
      { kind: "toolUse", id: "t1", name: "Bash", input: '{"command":"ls"}' },
      { kind: "toolResult", toolUseId: "t1", content: "a.ts", isError: false },
      { kind: "assistantText", text: "Done." },
      { kind: "turnDone", costUsd: 0.02, numTurns: 1 },
    ];
    const view = reduceAll(events, initialSessionView("starting"));
    expect(kinds(view)).toEqual(["assistant", "tool", "assistant"]);
    expect(view.claudeSessionId).toBe("sess-x");
    expect(view.model).toBe("claude-opus-4-8");
    expect(view.state).toBe("running");
    expect(view.streaming).toBe(false);
    expect(isBusy(view)).toBe(false);
    // No item id collisions.
    expect(new Set(view.items.map((i) => i.id)).size).toBe(view.items.length);
  });

  it("does not mutate the input view", () => {
    const before = initialSessionView("running");
    const frozen = JSON.stringify(before);
    reduceEvent(before, { kind: "partialText", text: "x" });
    expect(JSON.stringify(before)).toBe(frozen);
  });
});

describe("derived helpers", () => {
  it("isBusy reflects starting / streaming / awaiting, never terminal", () => {
    expect(isBusy({ ...initialSessionView("starting") })).toBe(true);
    expect(isBusy({ ...initialSessionView("running"), streaming: true })).toBe(true);
    expect(isBusy({ ...initialSessionView("awaiting_permission") })).toBe(true);
    expect(isBusy({ ...initialSessionView("running"), streaming: false })).toBe(false);
    expect(isBusy({ ...initialSessionView("completed") })).toBe(false);
  });

  it("describeState refines running by streaming", () => {
    expect(describeState("running", true).label).toBe("working");
    expect(describeState("running", false).label).toBe("ready");
    expect(describeState("awaiting_permission", false).tone).toBe("warn");
    expect(describeState("crashed", false).tone).toBe("bad");
  });

  it("addUserMessage appends and flags a turn in flight", () => {
    let view = initialSessionView("running");
    view = addUserMessage(view, "do the thing", "research");
    expect(view.items[0]).toMatchObject({ kind: "user", text: "do the thing", stage: "research" });
    expect(view.streaming).toBe(true);
  });

  it("viewToText renders speakers and tools", () => {
    let view = initialSessionView("running");
    view = addUserMessage(view, "hi", null);
    view = reduceEvent(view, { kind: "assistantText", text: "hello" });
    const text = viewToText(view);
    expect(text).toContain("## You");
    expect(text).toContain("## Assistant");
    expect(text).toContain("hello");
  });
});
