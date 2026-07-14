import { describe, expect, it } from "vitest";

import type { SessionEvent } from "@/lib/bindings";
import {
  addUserMessage,
  buildQuestionAnswers,
  cancelPendingPrompts,
  describeState,
  hasPendingPermission,
  initialSessionView,
  isBusy,
  isCompactSummary,
  isTerminal,
  isWorking,
  parkedState,
  parseQuestions,
  reduceEvent,
  resolvePermission,
  resolveQuestion,
  restoredView,
  splitInlineThinking,
  viewToText,
  type Question,
  type SessionView,
  type TranscriptItem,
} from "./transcript";

const TABS_QUESTION = JSON.stringify([
  {
    question: "Tabs or spaces?",
    header: "Indentation",
    multiSelect: false,
    options: [
      { label: "Tabs", description: "Tab characters." },
      { label: "Spaces", description: "Space characters." },
    ],
  },
]);

function reduceAll(events: SessionEvent[], start = initialSessionView("starting")): SessionView {
  return events.reduce(reduceEvent, start);
}

function kinds(view: SessionView): TranscriptItem["kind"][] {
  return view.items.map((i) => i.kind);
}

describe("text coalescing", () => {
  it("accumulates partials into one streaming block, then the final replaces it", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "partialText", text: "Hel", parentId: null });
    view = reduceEvent(view, { kind: "partialText", text: "lo wor", parentId: null });
    expect(view.items).toHaveLength(1);
    expect(view.items[0]).toMatchObject({ kind: "assistant", text: "Hello wor", streaming: true });

    view = reduceEvent(view, { kind: "assistantText", text: "Hello world.", parentId: null });
    expect(view.items).toHaveLength(1);
    expect(view.items[0]).toMatchObject({ kind: "assistant", text: "Hello world.", streaming: false });
  });

  it("creates a finalized block when assistantText arrives with no partials", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "assistantText", text: "Direct.", parentId: null });
    expect(view.items).toHaveLength(1);
    expect(view.items[0]).toMatchObject({ kind: "assistant", text: "Direct.", streaming: false });
  });

  it("keeps accumulated text when the final block is empty", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "partialText", text: "streamed", parentId: null });
    view = reduceEvent(view, { kind: "assistantText", text: "", parentId: null });
    expect(view.items[0]).toMatchObject({ text: "streamed", streaming: false });
  });

  it("ignores empty finals and empty partials with nothing open", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "assistantText", text: "", parentId: null });
    view = reduceEvent(view, { kind: "partialText", text: "", parentId: null });
    expect(view.items).toHaveLength(0);
  });

  it("starts a fresh block after a tool interrupts the text", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "partialText", text: "first", parentId: null });
    view = reduceEvent(view, { kind: "assistantText", text: "first", parentId: null });
    view = reduceEvent(view, { kind: "toolUse", id: "t1", name: "Read", input: "{}", parentId: null });
    view = reduceEvent(view, { kind: "partialText", text: "second", parentId: null });
    expect(kinds(view)).toEqual(["assistant", "tool", "assistant"]);
    expect(view.items[2]).toMatchObject({ text: "second", streaming: true });
  });
});

describe("launched skill bodies", () => {
  const SKILL_BODY =
    "Base directory for this skill: /Users/jt/.claude/skills/describe_pr\n\n# Generate PR Description\nYou are ...";

  it("folds the injected body into the Skill invocation row, not a second block", () => {
    let view = initialSessionView("running");
    // Real flow: the assistant says it'll run the skill, the Skill tool runs,
    // then its SKILL.md arrives as an assistant block right after the result.
    view = reduceEvent(view, { kind: "assistantText", text: "I'll run the doctor skill.", parentId: null });
    view = reduceEvent(view, {
      kind: "toolUse",
      id: "s1",
      name: "Skill",
      input: '{"args":"24","skill":"describe_pr"}',
      parentId: null,
    });
    view = reduceEvent(view, {
      kind: "toolResult",
      toolUseId: "s1",
      content: "Launching skill: describe_pr",
      isError: false,
      parentId: null,
    });
    view = reduceEvent(view, { kind: "assistantText", text: SKILL_BODY, parentId: null });

    // One prose block + one tool row — the body folds into the row, no extra block.
    expect(kinds(view)).toEqual(["assistant", "tool"]);
    expect(view.items[1]).toMatchObject({ kind: "tool", name: "Skill", skillBody: SKILL_BODY });
  });

  it("folds only into the matching skill row when several run in a row", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "toolUse", id: "s1", name: "Skill", input: '{"skill":"a"}', parentId: null });
    view = reduceEvent(view, { kind: "toolUse", id: "s2", name: "Skill", input: '{"skill":"b"}', parentId: null });
    view = reduceEvent(view, { kind: "assistantText", text: `${SKILL_BODY} (one)`, parentId: null });
    view = reduceEvent(view, { kind: "assistantText", text: `${SKILL_BODY} (two)`, parentId: null });
    // Bodies attach to the earliest still-awaiting row first, in order.
    expect(view.items[0]).toMatchObject({ toolUseId: "s1", skillBody: `${SKILL_BODY} (one)` });
    expect(view.items[1]).toMatchObject({ toolUseId: "s2", skillBody: `${SKILL_BODY} (two)` });
    expect(kinds(view)).toEqual(["tool", "tool"]);
  });

  it("falls back to a collapsed chip when a streamed body has no invocation row", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "partialText", text: "Base directory for this skill: ", parentId: null });
    expect(kinds(view)).toEqual(["assistant"]);
    view = reduceEvent(view, { kind: "assistantText", text: SKILL_BODY, parentId: null });
    expect(kinds(view)).toEqual(["skill"]);
    expect(view.items[0]).toMatchObject({ kind: "skill", name: "describe_pr" });
  });

  it("leaves ordinary prose that merely mentions the prefix as an assistant block", () => {
    let view = initialSessionView("running");
    const prose = 'A skill body starts with `Base directory for this skill:` — note that.';
    view = reduceEvent(view, { kind: "assistantText", text: prose, parentId: null });
    expect(kinds(view)).toEqual(["assistant"]);
  });
});

describe("tool blocks", () => {
  it("pairs a tool_result with its tool_use by id", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "toolUse", id: "abc", name: "Bash", input: '{"command":"ls"}', parentId: null });
    expect(view.items[0]).toMatchObject({ kind: "tool", toolUseId: "abc", result: null });

    view = reduceEvent(view, { kind: "toolResult", toolUseId: "abc", content: "file.txt", isError: false, parentId: null });
    expect(view.items).toHaveLength(1);
    expect(view.items[0]).toMatchObject({
      kind: "tool",
      result: { content: "file.txt", isError: false },
    });
  });

  it("surfaces an orphan tool_result as its own block", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "toolResult", toolUseId: "ghost", content: "x", isError: true, parentId: null });
    expect(view.items).toHaveLength(1);
    expect(view.items[0]).toMatchObject({ kind: "tool", name: "result", result: { isError: true } });
  });

  it("matches the right tool when several are open", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "toolUse", id: "t1", name: "Read", input: "{}", parentId: null });
    view = reduceEvent(view, { kind: "toolUse", id: "t2", name: "Grep", input: "{}", parentId: null });
    view = reduceEvent(view, { kind: "toolResult", toolUseId: "t1", content: "r1", isError: false, parentId: null });
    const t1 = view.items.find((i) => i.kind === "tool" && i.toolUseId === "t1");
    const t2 = view.items.find((i) => i.kind === "tool" && i.toolUseId === "t2");
    expect(t1).toMatchObject({ result: { content: "r1" } });
    expect(t2).toMatchObject({ result: null });
  });

  it("an AskUserQuestion result clears the pending question (it was answered)", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "toolUse", id: "auq1", name: "AskUserQuestion", input: "{}", parentId: null });
    view = reduceEvent(view, { kind: "questionAsk", requestId: "q-1", questions: TABS_QUESTION });
    expect(view.state).toBe("awaiting_question");

    view = reduceEvent(view, { kind: "toolResult", toolUseId: "auq1", content: "answered", isError: false, parentId: null });
    expect(view.pendingQuestion).toBeNull();
    expect(view.state).toBe("running");
  });

  it("a gated tool's result resolves its open permission card (it was answered)", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "toolUse", id: "t1", name: "Bash", input: "{}", parentId: null });
    view = reduceEvent(view, { kind: "permissionAsk", requestId: "req-1", toolName: "Bash", input: "{}" });
    expect(view.state).toBe("awaiting_permission");

    view = reduceEvent(view, { kind: "toolResult", toolUseId: "t1", content: "ok", isError: false, parentId: null });
    expect(hasPendingPermission(view)).toBe(false);
    expect(view.pendingPermission).toBeNull();
    expect(view.state).toBe("running");
  });

  it("a sibling tool's result leaves an unrelated open permission card intact", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "toolUse", id: "t1", name: "Read", input: "{}", parentId: null });
    view = reduceEvent(view, { kind: "permissionAsk", requestId: "req-1", toolName: "Bash", input: "{}" });
    // A different tool finishing must not un-park a Bash permission.
    view = reduceEvent(view, { kind: "toolResult", toolUseId: "t1", content: "ok", isError: false, parentId: null });
    expect(hasPendingPermission(view)).toBe(true);
    expect(view.state).toBe("awaiting_permission");
  });
});

describe("thinking", () => {
  it("adds a thinking block and closes any open assistant text", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "partialText", text: "hmm", parentId: null });
    view = reduceEvent(view, { kind: "thinking", text: "reasoning", parentId: null });
    expect(kinds(view)).toEqual(["assistant", "thinking"]);
    expect(view.items[0]).toMatchObject({ kind: "assistant", streaming: false });
  });
});

describe("OpenCode: late final text coalesces (no duplicate around tools)", () => {
  it("does not duplicate the intro when its text part ends after the tool starts", () => {
    // From a live `opencode serve` trace: the intro's text part gets its `time.end`
    // only *after* the tool, so `toolUse` closes the streamed block and the final
    // `assistantText` lands with nothing open — it must reunite, not duplicate it.
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "partialText", text: "I'm going to run the echo command.", parentId: null });
    view = reduceEvent(view, { kind: "toolUse", id: "t1", name: "Bash", input: '{"command":"echo hi"}', parentId: null });
    view = reduceEvent(view, { kind: "assistantText", text: "I'm going to run the echo command.", parentId: null });

    expect(kinds(view)).toEqual(["assistant", "tool"]);
    expect(view.items[0]).toMatchObject({ kind: "assistant", text: "I'm going to run the echo command.", streaming: false });
  });

  it("completes a stream that was cut off mid-part by the tool", () => {
    // The stream was closed by the tool before the last delta; the longer final
    // still coalesces into the streamed block (prefix match) rather than dupes.
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "partialText", text: "Reading the", parentId: null });
    view = reduceEvent(view, { kind: "toolUse", id: "t1", name: "Read", input: "{}", parentId: null });
    view = reduceEvent(view, { kind: "assistantText", text: "Reading the config file.", parentId: null });

    expect(kinds(view)).toEqual(["assistant", "tool"]);
    expect(view.items[0]).toMatchObject({ text: "Reading the config file.", streaming: false });
  });

  it("still appends genuinely new prose after a tool", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "partialText", text: "Looking…", parentId: null });
    view = reduceEvent(view, { kind: "assistantText", text: "Looking…", parentId: null });
    view = reduceEvent(view, { kind: "toolUse", id: "t1", name: "Bash", input: "{}", parentId: null });
    view = reduceEvent(view, { kind: "assistantText", text: "All done.", parentId: null });
    expect(kinds(view)).toEqual(["assistant", "tool", "assistant"]);
    expect(view.items[2]).toMatchObject({ text: "All done." });
  });
});

describe("OpenCode: inline <think> reasoning is split out of prose", () => {
  it("splits a <think>…</think> prefix into a thinking block + clean answer", () => {
    let view = initialSessionView("running");
    const raw = "<think>Check divisibility of 91.</think>No, 91 = 7 × 13.";
    view = reduceEvent(view, { kind: "partialText", text: raw, parentId: null });
    view = reduceEvent(view, { kind: "assistantText", text: raw, parentId: null });
    expect(kinds(view)).toEqual(["thinking", "assistant"]);
    expect(view.items[0]).toMatchObject({ kind: "thinking", text: "Check divisibility of 91." });
    expect(view.items[1]).toMatchObject({ kind: "assistant", text: "No, 91 = 7 × 13.", streaming: false });
  });

  it("treats an orphaned leading </think> (stripped opener) as reasoning", () => {
    // The exact shape in the reported transcript: reasoning, a bare </think>, answer.
    let view = initialSessionView("running");
    const raw = "We need to find the homepage. Let's list src. </think>I'll add it near the bottom.";
    view = reduceEvent(view, { kind: "assistantText", text: raw, parentId: null });
    expect(kinds(view)).toEqual(["thinking", "assistant"]);
    expect(view.items[0]).toMatchObject({ kind: "thinking", text: "We need to find the homepage. Let's list src." });
    expect(view.items[1]).toMatchObject({ text: "I'll add it near the bottom." });
    // The raw closing tag never reaches the rendered/copied transcript.
    expect(viewToText(view)).not.toContain("</think>");
  });

  it("places the thought where the text streamed — before a tool it preceded", () => {
    // Reasoning leaked inline, the block was closed by a tool, then the final lands.
    let view = initialSessionView("running");
    const raw = "<think>Plan the edit.</think>Editing now.";
    view = reduceEvent(view, { kind: "partialText", text: raw, parentId: null });
    view = reduceEvent(view, { kind: "toolUse", id: "t1", name: "Edit", input: "{}", parentId: null });
    view = reduceEvent(view, { kind: "assistantText", text: raw, parentId: null });
    expect(kinds(view)).toEqual(["thinking", "assistant", "tool"]);
    expect(view.items[0]).toMatchObject({ kind: "thinking", text: "Plan the edit." });
    expect(view.items[1]).toMatchObject({ kind: "assistant", text: "Editing now." });
  });

  it("emits only a thinking block when the part is all reasoning", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "assistantText", text: "<think>Still working on it.</think>", parentId: null });
    expect(kinds(view)).toEqual(["thinking"]);
    expect(view.items[0]).toMatchObject({ kind: "thinking", text: "Still working on it." });
  });

  it("splitInlineThinking handles pairs, orphans, unclosed, and plain prose", () => {
    expect(splitInlineThinking("plain answer")).toEqual({ reasoning: null, answer: "plain answer" });
    expect(splitInlineThinking("<think>R</think>A")).toEqual({ reasoning: "R", answer: "A" });
    expect(splitInlineThinking("<thinking>R</thinking>A")).toEqual({ reasoning: "R", answer: "A" });
    expect(splitInlineThinking("R</think>A")).toEqual({ reasoning: "R", answer: "A" });
    expect(splitInlineThinking("<think>R only")).toEqual({ reasoning: "R only", answer: "" });
    // Answer prose that genuinely precedes a real think pair is preserved.
    expect(splitInlineThinking("A1<think>R</think>A2")).toEqual({ reasoning: "R", answer: "A1A2" });
  });
});

describe("subagent nesting", () => {
  it("does not duplicate the main block when subagent steps interleave its stream", () => {
    let view = initialSessionView("running");
    // Main agent starts streaming a progress message…
    view = reduceEvent(view, { kind: "partialText", text: "7 agents running. ", parentId: null });
    // …a parallel subagent emits a step, landing after the open main block…
    view = reduceEvent(view, { kind: "toolUse", id: "s1", name: "Read", input: "{}", parentId: "task1" });
    // …the main agent keeps streaming, then finalizes.
    view = reduceEvent(view, { kind: "partialText", text: "Waiting.", parentId: null });
    view = reduceEvent(view, { kind: "assistantText", text: "7 agents running. Waiting.", parentId: null });

    const mainAssistants = view.items.filter(
      (i) => i.kind === "assistant" && !("parentId" in i && i.parentId),
    );
    expect(mainAssistants).toHaveLength(1);
    expect(mainAssistants[0]).toMatchObject({ text: "7 agents running. Waiting.", streaming: false });
  });

  it("tags subagent items with parentId and suppresses their partials", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, {
      kind: "toolUse",
      id: "task1",
      name: "Task",
      input: '{"description":"research"}',
      parentId: null,
    });
    // Subagent activity, all parented to the Task.
    view = reduceEvent(view, { kind: "partialText", text: "streaming…", parentId: "task1" });
    view = reduceEvent(view, { kind: "assistantText", text: "sub answer", parentId: "task1" });
    view = reduceEvent(view, { kind: "toolUse", id: "s1", name: "Read", input: "{}", parentId: "task1" });
    view = reduceEvent(view, {
      kind: "toolResult",
      toolUseId: "s1",
      content: "data",
      isError: false,
      parentId: "task1",
    });
    // The Task's own result comes back on the main thread.
    view = reduceEvent(view, {
      kind: "toolResult",
      toolUseId: "task1",
      content: "report",
      isError: false,
      parentId: null,
    });

    const task = view.items.find((i) => i.kind === "tool" && i.toolUseId === "task1");
    expect(task).toMatchObject({ result: { content: "report" }, parentId: null });
    // Subagent text + tool are tagged with the parent and stay out of the main flow.
    const subItems = view.items.filter(
      (i): i is Extract<TranscriptItem, { parentId?: string | null }> =>
        "parentId" in i && i.parentId === "task1",
    );
    expect(subItems.map((i) => i.kind)).toEqual(["assistant", "tool"]);
    // The suppressed subagent partial created no main-thread assistant block.
    const mainAssistants = view.items.filter(
      (i) => i.kind === "assistant" && !("parentId" in i && i.parentId),
    );
    expect(mainAssistants).toHaveLength(0);
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

  it("stays parked across parallel asks until the last is answered", () => {
    // A subagent firing parallel Bash calls produces several open asks at once.
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "permissionAsk", requestId: "req-1", toolName: "Bash", input: "{}" });
    view = reduceEvent(view, { kind: "permissionAsk", requestId: "req-2", toolName: "Bash", input: "{}" });
    expect(isBusy(view)).toBe(true);

    view = resolvePermission(view, "req-1", "allow");
    expect(view.state).toBe("awaiting_permission");
    expect(hasPendingPermission(view)).toBe(true);
    expect(isBusy(view)).toBe(true);

    view = resolvePermission(view, "req-2", "allow");
    expect(view.state).toBe("running");
    expect(hasPendingPermission(view)).toBe(false);
  });

  it("a turnDone while an ask is open keeps the session parked, not idle", () => {
    // Regression: a subagent finishing emitted turnDone and dropped the session
    // to "running"/not-busy while permission cards were still unanswered, so the
    // stop button vanished and the composer looked ready mid-block.
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "permissionAsk", requestId: "req-1", toolName: "Bash", input: "{}" });
    view = reduceEvent(view, { kind: "turnDone", costUsd: null, numTurns: null, contextTokens: null });
    expect(view.state).toBe("awaiting_permission");
    expect(isBusy(view)).toBe(true);
  });

  it("cancelPendingPrompts denies every open card and clears both pending prompts", () => {
    // The escape hatch when a turn is abandoned (permission-mode restart / Stop)
    // rather than answered: the parked state must clear or the session is stuck
    // `isBusy` forever on an ask no live claude can resolve.
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "permissionAsk", requestId: "req-1", toolName: "Bash", input: "{}" });
    view = reduceEvent(view, { kind: "permissionAsk", requestId: "req-2", toolName: "Bash", input: "{}" });
    view = reduceEvent(view, { kind: "questionAsk", requestId: "q-1", questions: TABS_QUESTION });
    expect(isBusy(view)).toBe(true);

    view = cancelPendingPrompts(view);
    expect(view.pendingPermission).toBeNull();
    expect(view.pendingQuestion).toBeNull();
    expect(hasPendingPermission(view)).toBe(false);
    expect(view.items.every((it) => it.kind !== "permission" || it.decision === "deny")).toBe(true);
    // Nothing is parked anymore, so once the caller settles state the turn is free.
    expect(isBusy({ ...view, state: parkedState(view), streaming: false })).toBe(false);
  });
});

describe("question flow", () => {
  it("parks the session on a parsed AskUserQuestion (no transcript card)", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "questionAsk", requestId: "q-1", questions: TABS_QUESTION });
    expect(view.state).toBe("awaiting_question");
    expect(isBusy(view)).toBe(true);
    expect(view.pendingQuestion).toEqual({
      requestId: "q-1",
      questions: [
        {
          question: "Tabs or spaces?",
          header: "Indentation",
          multiSelect: false,
          options: [
            { label: "Tabs", description: "Tab characters." },
            { label: "Spaces", description: "Space characters." },
          ],
        },
      ],
    });
    // The questions render above the composer, not as a transcript item.
    expect(view.items).toHaveLength(0);
  });

  it("resolveQuestion clears the pending question and resumes", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "questionAsk", requestId: "q-1", questions: TABS_QUESTION });
    view = resolveQuestion(view, "q-1");
    expect(view.pendingQuestion).toBeNull();
    expect(view.state).toBe("running");
    expect(view.streaming).toBe(true);
  });

  it("a stale resolve leaves an unrelated pending question intact", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "questionAsk", requestId: "q-2", questions: TABS_QUESTION });
    view = resolveQuestion(view, "q-1");
    expect(view.pendingQuestion?.requestId).toBe("q-2");
    expect(view.state).toBe("awaiting_question");
  });

  it("a turnDone while a question is open keeps the session parked, not idle", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "questionAsk", requestId: "q-1", questions: TABS_QUESTION });
    view = reduceEvent(view, { kind: "turnDone", costUsd: null, numTurns: null, contextTokens: null });
    expect(view.state).toBe("awaiting_question");
    expect(isBusy(view)).toBe(true);
  });

  it("exit clears a pending question", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "questionAsk", requestId: "q-1", questions: TABS_QUESTION });
    view = reduceEvent(view, { kind: "exited", outcome: "killed", code: null, stderrTail: "" });
    expect(view.pendingQuestion).toBeNull();
    expect(isBusy(view)).toBe(false);
  });

  it("parseQuestions drops malformed payloads instead of throwing", () => {
    expect(parseQuestions("not json")).toEqual([]);
    expect(parseQuestions(JSON.stringify({ not: "an array" }))).toEqual([]);
    // A question with no text is skipped; valid siblings survive.
    const mixed = JSON.stringify([
      { header: "x", options: [] },
      { question: "Real?", options: [{ label: "Yes" }, { bogus: true }] },
    ]);
    expect(parseQuestions(mixed)).toEqual([
      { question: "Real?", header: "", multiSelect: false, options: [{ label: "Yes", description: "" }] },
    ]);
  });

  it("buildQuestionAnswers merges option picks with custom 'Other' text", () => {
    const single: Question = { question: "Color?", header: "", multiSelect: false, options: [] };
    const multi: Question = { question: "Langs?", header: "", multiSelect: true, options: [] };

    // Single-select via a preset pick.
    expect(buildQuestionAnswers([single], { "Color?": ["Red"] }, {})).toEqual({ "Color?": "Red" });

    // Single-select via "Other" only (no pick) — the custom text is the answer.
    expect(buildQuestionAnswers([single], {}, { "Color?": "Chartreuse" })).toEqual({
      "Color?": "Chartreuse",
    });

    // Multi-select appends the custom text after the picks.
    expect(
      buildQuestionAnswers([multi], { "Langs?": ["Rust", "TS"] }, { "Langs?": "Zig" }),
    ).toEqual({ "Langs?": "Rust, TS, Zig" });

    // Custom text is trimmed; whitespace-only and absent answers are omitted.
    expect(
      buildQuestionAnswers([single, multi], {}, { "Color?": "  Teal  ", "Langs?": "   " }),
    ).toEqual({ "Color?": "Teal" });

    // Nothing chosen anywhere → empty map (the "dismissed" signal).
    expect(buildQuestionAnswers([single, multi], {}, {})).toEqual({});
  });

  it("describeState labels a pending question as needing an answer", () => {
    expect(describeState("awaiting_question", true)).toEqual({ label: "needs answer", tone: "warn" });
  });
});

describe("turn + exit lifecycle", () => {
  it("turnDone closes streaming and records cumulative cost/turns", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "partialText", text: "answer", parentId: null });
    view = reduceEvent(view, {
      kind: "turnDone",
      costUsd: 0.0123,
      numTurns: 2,
      contextTokens: 24130,
    });
    expect(view.streaming).toBe(false);
    expect(view.state).toBe("running");
    expect(view.costUsd).toBeCloseTo(0.0123);
    expect(view.numTurns).toBe(2);
    expect(view.contextTokens).toBe(24130);
    expect(view.items[0]).toMatchObject({ kind: "assistant", streaming: false });
  });

  it("contextWindow sets the meter denominator without touching anything else", () => {
    // The backend reports a model's real window (e.g. an Ollama model's fixed
    // num_ctx) so the meter doesn't fall back to the Claude 1M default.
    let view = initialSessionView("running");
    expect(view.contextWindow).toBeNull();
    view = reduceEvent(view, { kind: "contextWindow", tokens: 32768 });
    expect(view.contextWindow).toBe(32768);
    expect(view.state).toBe("running");
    expect(view.items).toHaveLength(0);
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

  it("commandLifecycle draws a seam on started, naming the wakeup and reusing its reason", () => {
    let view = initialSessionView("running");
    // The agent schedules a wakeup, then ends its turn.
    view = reduceEvent(view, {
      kind: "toolUse",
      id: "t1",
      name: "ScheduleWakeup",
      input: '{"delaySeconds":75,"prompt":"Check the deploy","reason":"waiting for Vercel"}',
      parentId: null,
    });
    view = reduceEvent(view, { kind: "partialText", text: "waiting…", parentId: null });
    // …the wakeup fires: claude injects the queued prompt and a new turn begins.
    view = reduceEvent(view, {
      kind: "commandLifecycle",
      commandUuid: "cd3eda46",
      state: "started",
    });
    // The seam says what the wakeup was for, rather than inventing copy — the detail
    // is the scheduling call's own `reason`.
    expect(view.items[view.items.length - 1]).toMatchObject({
      kind: "queuedCommand",
      label: "Scheduled wakeup",
      detail: "waiting for Vercel",
    });
    // The open prose block is closed so the seam doesn't land mid-stream.
    expect(view.items.find((i) => i.kind === "assistant")).toMatchObject({ streaming: false });
  });

  it("commandLifecycle falls back to the wakeup's prompt when it carries no reason", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, {
      kind: "toolUse",
      id: "t1",
      name: "ScheduleWakeup",
      input: '{"delaySeconds":60,"prompt":"Re-check the build"}',
      parentId: null,
    });
    view = reduceEvent(view, { kind: "commandLifecycle", commandUuid: "c1", state: "started" });
    expect(view.items[view.items.length - 1]).toMatchObject({
      label: "Scheduled wakeup",
      detail: "Re-check the build",
    });
  });

  it("commandLifecycle labels a plain queued command generically, with no detail", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, {
      kind: "toolUse",
      id: "t1",
      name: "Bash",
      input: '{"command":"ls"}',
      parentId: null,
    });
    view = reduceEvent(view, { kind: "commandLifecycle", commandUuid: "c1", state: "started" });
    expect(view.items[view.items.length - 1]).toMatchObject({
      kind: "queuedCommand",
      label: "Queued command",
      detail: null,
    });
  });

  it("commandLifecycle close brackets render nothing — only the opening seam is drawn", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "commandLifecycle", commandUuid: "c1", state: "started" });
    const afterStart = view.items.length;
    for (const state of ["completed", "cancelled"]) {
      view = reduceEvent(view, { kind: "commandLifecycle", commandUuid: "c1", state });
    }
    expect(view.items.length).toBe(afterStart);
    expect(view.items.filter((i) => i.kind === "queuedCommand")).toHaveLength(1);
  });

  it("compactBoundary appends a divider, closes the open assistant, and keeps the turn streaming", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "partialText", text: "thinking…", parentId: null });
    expect(view.items[0]).toMatchObject({ kind: "assistant", streaming: true });
    view = reduceEvent(view, { kind: "compactBoundary", trigger: "auto", preTokens: 182_431 });
    // The open assistant block is closed so the divider doesn't land mid-stream,
    // but a turn that was in flight (`streaming: true`) stays in flight — the
    // post-compact turn keeps the spinner alive until its own `turnDone`.
    expect(view.items[0]).toMatchObject({ kind: "assistant", streaming: false });
    expect(view.items[1]).toMatchObject({ kind: "compact", trigger: "auto", preTokens: 182_431 });
    expect(view.streaming).toBe(true);
    expect(view.state).toBe("running");
  });
});

describe("post-compact summary", () => {
  // claude delivers the summary as one big assistant block right after the
  // boundary; it always opens with this system-injected preamble.
  const SUMMARY =
    "This session is being continued from a previous conversation that ran out of context. " +
    "The summary below covers the earlier portion of the conversation.\n\nSummary:\n1. Primary Request…";

  it("isCompactSummary matches the preamble, not ordinary prose", () => {
    expect(isCompactSummary(SUMMARY)).toBe(true);
    expect(isCompactSummary("This session is being continued from a previous conversation")).toBe(true);
    expect(isCompactSummary("Here is a summary of the previous conversation.")).toBe(false);
    expect(isCompactSummary("")).toBe(false);
  });

  it("folds the summary into the preceding compact divider, not a new block", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "compactBoundary", trigger: "auto", preTokens: 167_335 });
    view = reduceEvent(view, { kind: "assistantText", text: SUMMARY, parentId: null });
    // Just the divider — the 12–18KB summary folds into it, never dumped inline.
    expect(kinds(view)).toEqual(["compact"]);
    expect(view.items[0]).toMatchObject({ kind: "compact", trigger: "auto", summary: SUMMARY });
    // The turn is still in flight; its real continuation renders as normal prose.
    view = reduceEvent(view, { kind: "assistantText", text: "Now I'll read the file.", parentId: null });
    expect(kinds(view)).toEqual(["compact", "assistant"]);
    expect(view.items[1]).toMatchObject({ kind: "assistant", text: "Now I'll read the file." });
  });

  it("drops a streamed partial summary so it isn't shown twice", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "compactBoundary", trigger: "manual", preTokens: null });
    // If the summary streamed as partials first, an open assistant block holds
    // them — folding the completed summary in must remove that partial block.
    view = reduceEvent(view, { kind: "partialText", text: SUMMARY.slice(0, 40), parentId: null });
    expect(kinds(view)).toEqual(["compact", "assistant"]);
    view = reduceEvent(view, { kind: "assistantText", text: SUMMARY, parentId: null });
    expect(kinds(view)).toEqual(["compact"]);
    expect(view.items[0]).toMatchObject({ kind: "compact", summary: SUMMARY });
  });

  it("leaves a summary-prefixed block as prose when no boundary is awaiting one", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "assistantText", text: SUMMARY, parentId: null });
    // No compact divider to fold into → render it rather than silently drop it.
    expect(kinds(view)).toEqual(["assistant"]);
    expect(view.items[0]).toMatchObject({ kind: "assistant", text: SUMMARY });
  });

  it("copies a compaction marker but not the bulky summary text", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "compactBoundary", trigger: "auto", preTokens: 167_335 });
    view = reduceEvent(view, { kind: "assistantText", text: SUMMARY, parentId: null });
    const text = viewToText(view);
    expect(text).toContain("Conversation compacted");
    expect(text).not.toContain("Primary Request");
  });
});

describe("connecting state (leaves 'starting' on first sign of life)", () => {
  it("init clears 'starting'", () => {
    const view = reduceEvent(initialSessionView("starting"), {
      kind: "init",
      claudeSessionId: "sess-x",
      model: "claude-opus-4-8",
    });
    expect(view.state).toBe("running");
  });

  it("streamed text clears 'starting' even with no init (a re-tailed live session)", () => {
    // After an HMR/UI reload the backend re-tails the still-live process without
    // re-emitting `init`; the next streamed event must drop the connecting state.
    let view = reduceEvent(initialSessionView("starting"), {
      kind: "partialText",
      text: "Resuming…",
      parentId: null,
    });
    expect(view.state).toBe("running");
    expect(view.streaming).toBe(true);

    view = reduceEvent(initialSessionView("starting"), {
      kind: "toolUse",
      id: "t1",
      name: "Edit",
      input: "{}",
      parentId: null,
    });
    expect(view.state).toBe("running");
  });

  it("a fresh 'starting' view with no events stays connecting", () => {
    // Nothing has streamed yet — the spinner should still show.
    expect(initialSessionView("starting").state).toBe("starting");
    expect(initialSessionView("starting").streaming).toBe(false);
  });
});

describe("relaunched process-boundary marker", () => {
  it("appends exactly one non-rendering 'relaunched' marker", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "assistantText", text: "before relaunch", parentId: null });
    view = reduceEvent(view, { kind: "relaunched" });

    expect(view.items.filter((i) => i.kind === "relaunched")).toHaveLength(1);
    // Non-rendering: the copied transcript is just the assistant block.
    expect(viewToText(view).trim()).toBe("## Assistant\nbefore relaunch");
  });
});

describe("replay determinism", () => {
  it("rebuilds the same transcript from a recorded stream", () => {
    const events: SessionEvent[] = [
      { kind: "init", claudeSessionId: "sess-x", model: "claude-opus-4-8" },
      { kind: "partialText", text: "Look", parentId: null },
      { kind: "partialText", text: "ing…", parentId: null },
      { kind: "assistantText", text: "Looking…", parentId: null },
      { kind: "toolUse", id: "t1", name: "Bash", input: '{"command":"ls"}', parentId: null },
      { kind: "toolResult", toolUseId: "t1", content: "a.ts", isError: false, parentId: null },
      { kind: "assistantText", text: "Done.", parentId: null },
      { kind: "turnDone", costUsd: 0.02, numTurns: 1, contextTokens: 12000 },
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
    reduceEvent(before, { kind: "partialText", text: "x", parentId: null });
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

  it("isBusy stays true while a permission is unanswered, even if state looks idle", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "permissionAsk", requestId: "req-1", toolName: "Bash", input: "{}" });
    // Force the not-busy shape a stray turnDone would have left behind.
    view = { ...view, state: "running", streaming: false };
    expect(hasPendingPermission(view)).toBe(true);
    expect(isBusy(view)).toBe(true);
  });

  it("isWorking is true only while computing — not while parked on the user", () => {
    expect(isWorking({ ...initialSessionView("starting") })).toBe(true);
    expect(isWorking({ ...initialSessionView("running"), streaming: true })).toBe(true);
    expect(isWorking({ ...initialSessionView("running"), streaming: false })).toBe(false);
    // Parked on a permission/question is busy but NOT working (blocked on you),
    // even though the reducer leaves `streaming` true on the ask.
    expect(isWorking({ ...initialSessionView("awaiting_permission"), streaming: true })).toBe(false);
    expect(isWorking({ ...initialSessionView("awaiting_question"), streaming: true })).toBe(false);
    expect(isWorking({ ...initialSessionView("completed") })).toBe(false);
  });

  it("isWorking ignores a stray streaming flag while a permission card is open", () => {
    let view = initialSessionView("running");
    view = reduceEvent(view, { kind: "permissionAsk", requestId: "req-1", toolName: "Bash", input: "{}" });
    view = { ...view, state: "running", streaming: true }; // stray turnDone shape
    expect(hasPendingPermission(view)).toBe(true);
    expect(isWorking(view)).toBe(false);
  });
});

describe("restoredView (rebuild after an app restart)", () => {
  it("holds a session parked on a permission instead of faking a running turn", () => {
    // Replay a log that ends mid-turn on a permission ask (the app was closed
    // before the user answered).
    let view = initialSessionView("starting");
    view = reduceEvent(view, { kind: "init", claudeSessionId: "c1", model: "claude-opus-4-8" });
    view = reduceEvent(view, { kind: "toolUse", id: "t1", name: "Bash", input: "{}", parentId: null });
    view = reduceEvent(view, { kind: "permissionAsk", requestId: "req-1", toolName: "Bash", input: "{}" });

    const restored = restoredView(view);

    // HELD: the conversation pauses for the user, the card stays answerable…
    expect(restored.state).toBe("awaiting_permission");
    expect(hasPendingPermission(restored)).toBe(true);
    expect(restored.pendingPermission?.requestId).toBe("req-1");
    expect(isBusy(restored)).toBe(true);
    // …but it is NOT shown as a running turn — this is the restart bug.
    expect(restored.streaming).toBe(false);
    expect(isWorking(restored)).toBe(false);
  });

  it("holds a session parked on a question", () => {
    let view = initialSessionView("starting");
    view = reduceEvent(view, { kind: "questionAsk", requestId: "q-1", questions: TABS_QUESTION });
    const restored = restoredView(view);
    expect(restored.state).toBe("awaiting_question");
    expect(restored.pendingQuestion?.requestId).toBe("q-1");
    expect(isWorking(restored)).toBe(false);
  });

  it("settles a session cut off mid-stream to idle (no fake working)", () => {
    let view = initialSessionView("starting");
    view = reduceEvent(view, { kind: "init", claudeSessionId: "c1", model: "m" });
    view = reduceEvent(view, { kind: "partialText", text: "thinking…", parentId: null });
    expect(view.streaming).toBe(true);

    const restored = restoredView(view);
    expect(restored.state).toBe("idle");
    expect(restored.streaming).toBe(false);
    expect(isBusy(restored)).toBe(false);
    expect(isWorking(restored)).toBe(false);
  });

  it("leaves a terminally-exited session terminal", () => {
    let view = initialSessionView("starting");
    view = reduceEvent(view, { kind: "init", claudeSessionId: "c1", model: "m" });
    view = reduceEvent(view, { kind: "exited", outcome: "completed", code: 0, stderrTail: "" });
    const restored = restoredView(view);
    expect(restored.state).toBe("completed");
    expect(isTerminal(restored.state)).toBe(true);
    expect(isWorking(restored)).toBe(false);
  });

  it("does not re-show an already-answered question after a restart", () => {
    // Mirrors the bug report: two AskUserQuestion calls, both answered, then the
    // turn completes with a recap. On reload nothing should be pending and the
    // composer must not be locked on a stale question.
    let view = initialSessionView("starting");
    view = reduceEvent(view, { kind: "init", claudeSessionId: "c1", model: "m" });
    for (const [tool, req] of [
      ["auq1", "q-1"],
      ["auq2", "q-2"],
    ] as const) {
      view = reduceEvent(view, { kind: "toolUse", id: tool, name: "AskUserQuestion", input: "{}", parentId: null });
      view = reduceEvent(view, { kind: "questionAsk", requestId: req, questions: TABS_QUESTION });
      view = reduceEvent(view, { kind: "toolResult", toolUseId: tool, content: "answered", isError: false, parentId: null });
    }
    view = reduceEvent(view, { kind: "assistantText", text: "All five answered.", parentId: null });
    view = reduceEvent(view, { kind: "turnDone", costUsd: 0.1, numTurns: 1, contextTokens: 29400 });

    const restored = restoredView(view);
    expect(restored.pendingQuestion).toBeNull();
    expect(restored.state).toBe("idle");
    expect(isBusy(restored)).toBe(false);
  });

  it("does not re-show an already-answered permission after a restart", () => {
    let view = initialSessionView("starting");
    view = reduceEvent(view, { kind: "init", claudeSessionId: "c1", model: "m" });
    view = reduceEvent(view, { kind: "toolUse", id: "t1", name: "Bash", input: "{}", parentId: null });
    view = reduceEvent(view, { kind: "permissionAsk", requestId: "req-1", toolName: "Bash", input: "{}" });
    view = reduceEvent(view, { kind: "toolResult", toolUseId: "t1", content: "ok", isError: false, parentId: null });
    view = reduceEvent(view, { kind: "turnDone", costUsd: null, numTurns: 1, contextTokens: null });

    const restored = restoredView(view);
    expect(hasPendingPermission(restored)).toBe(false);
    expect(restored.pendingPermission).toBeNull();
    expect(restored.state).toBe("idle");
    expect(isBusy(restored)).toBe(false);
  });

  it("replays persisted user turns so the prompts survive a restart", () => {
    // The user's side of the conversation is persisted as `userMessage` events
    // (claude never echoes the prompt); replaying must rebuild both sides.
    let view = initialSessionView("starting");
    view = reduceEvent(view, { kind: "userMessage", text: "/research_codebase widgets", stage: "research" });
    view = reduceEvent(view, { kind: "init", claudeSessionId: "c1", model: "m" });
    view = reduceEvent(view, { kind: "assistantText", text: "Here's what I found…", parentId: null });
    view = reduceEvent(view, { kind: "turnDone", costUsd: null, numTurns: 1, contextTokens: null });
    view = reduceEvent(view, { kind: "userMessage", text: "now go deeper", stage: null });
    view = reduceEvent(view, { kind: "assistantText", text: "Going deeper.", parentId: null });
    view = reduceEvent(view, { kind: "turnDone", costUsd: null, numTurns: 2, contextTokens: null });

    const restored = restoredView(view);
    const speakers = restored.items.map((i) => i.kind);
    expect(speakers).toEqual(["user", "assistant", "user", "assistant"]);
    expect(restored.items[0]).toMatchObject({
      kind: "user",
      text: "/research_codebase widgets",
      stage: "research",
    });
    expect(restored.items[2]).toMatchObject({ kind: "user", text: "now go deeper", stage: null });
    expect(restored.state).toBe("idle");
  });

  it("replays a prompt run as a named card, keeping the body the agent got", () => {
    // The card is reconstructed from the persisted event, not from live state —
    // otherwise a restart would dump the whole prompt body back into the chat.
    let view = initialSessionView("starting");
    view = reduceEvent(view, {
      kind: "userMessage",
      text: "Refactor this module.\nStep 1…\nStep 2…",
      stage: null,
      promptName: "Refactor for testability",
    });

    expect(restoredView(view).items[0]).toMatchObject({
      kind: "user",
      text: "Refactor this module.\nStep 1…\nStep 2…",
      promptName: "Refactor for testability",
    });
  });

  it("replays a turn persisted before prompt cards existed", () => {
    // `promptName` is absent from those rows; it must read as a typed message,
    // not crash or render an untitled card.
    let view = initialSessionView("starting");
    view = reduceEvent(view, { kind: "userMessage", text: "hello", stage: null });

    expect(restoredView(view).items[0]).toMatchObject({
      kind: "user",
      text: "hello",
      promptName: null,
    });
  });
});

describe("display + actions", () => {
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
    view = reduceEvent(view, { kind: "assistantText", text: "hello", parentId: null });
    const text = viewToText(view);
    expect(text).toContain("## You");
    expect(text).toContain("## Assistant");
    expect(text).toContain("hello");
  });
});
