import { describe, expect, it } from "vitest";

import { activityStatus, nextTurn, recapWanted, type ActivityInput } from "./activityStatus";
import { initialSessionView, type TranscriptItem } from "./transcript";

type Tool = Extract<TranscriptItem, { kind: "tool" }>;

const ok = (content = "ok") => ({ content, isError: false });

function tool(name: string, input: Record<string, unknown>, id: string, opts: Partial<Tool> = {}): Tool {
  return { kind: "tool", id: `i-${id}`, toolUseId: id, name, input: JSON.stringify(input), result: null, ...opts };
}

const user = (id = "u1"): TranscriptItem => ({
  kind: "user",
  id,
  text: "go",
  stage: null,
  attachments: [],
  createdAt: null,
  promptName: null,
});

function entry(items: TranscriptItem[], over: Partial<ActivityInput> = {}): ActivityInput {
  return {
    ...initialSessionView("running"),
    streaming: true,
    items,
    workingSince: 1000,
    awaitingSince: null,
    turnAuthor: "self",
    lastTurn: null,
    recap: null,
    ...over,
  };
}

const ctx = { harnessLabel: "Claude Code", otherDevice: null };

describe("activityStatus", () => {
  it("is null with no session before start", () => {
    expect(activityStatus(undefined, { ...ctx, started: false })).toBeNull();
  });

  it("says it is connecting before the session attaches", () => {
    expect(activityStatus(undefined, { ...ctx, started: true })?.label).toBe("Connecting to Claude Code…");
    expect(activityStatus(entry([], { state: "starting", streaming: false }), ctx)?.label).toBe(
      "Connecting to Claude Code…",
    );
  });

  it("is null when idle between turns", () => {
    expect(activityStatus(entry([user()], { streaming: false, workingSince: null }), ctx)).toBeNull();
  });

  it("is null once the session has ended", () => {
    expect(activityStatus(entry([user()], { state: "completed", streaming: false }), ctx)).toBeNull();
    expect(activityStatus(entry([user()], { state: "crashed" }), ctx)).toBeNull();
  });

  it("waits on a pending question before anything the stream is doing", () => {
    const s = activityStatus(
      entry([user(), tool("Read", { file_path: "/a.ts" }, "t1")], {
        state: "awaiting_question",
        pendingQuestion: { requestId: "q", questions: [] },
        awaitingSince: 5000,
      }),
      ctx,
    );
    expect(s).toMatchObject({ label: "Waiting for your answer", tone: "wait", since: 5000 });
  });

  it("names the tool awaiting approval", () => {
    const s = activityStatus(
      entry([
        user(),
        { kind: "permission", id: "p", requestId: "r", toolName: "Bash", input: "{}", decision: null },
      ]),
      ctx,
    );
    expect(s).toMatchObject({ label: "Waiting for approval", detail: "Bash", tone: "wait" });
  });

  it("ignores a decided permission", () => {
    const s = activityStatus(
      entry([
        user(),
        { kind: "permission", id: "p", requestId: "r", toolName: "Bash", input: "{}", decision: "allow" },
      ]),
      ctx,
    );
    expect(s?.tone).toBe("work");
  });

  it("credits another device's turn to that device", () => {
    const s = activityStatus(entry([user()], { turnAuthor: "other" }), { ...ctx, otherDevice: "Jon's MacBook" });
    expect(s?.label).toBe("Jon's MacBook is working…");
  });

  it("times the whole turn, not just since the last approval", () => {
    expect(activityStatus(entry([user()], { turnSince: 400, workingSince: 1000 }), ctx)?.since).toBe(400);
  });

  it("names the other device while its turn waits for approval, without asking this user", () => {
    const s = activityStatus(
      entry(
        [user(), { kind: "permission", id: "p", requestId: "r", toolName: "Bash", input: "{}", decision: null }],
        { turnAuthor: "other", awaitingSince: 7 },
      ),
      { ...ctx, otherDevice: "nexus" },
    );
    expect(s).toMatchObject({ label: "nexus is waiting for approval", tone: "quiet", since: 7 });
  });

  it("names the other device while its turn waits on a question", () => {
    const s = activityStatus(
      entry([user()], { turnAuthor: "other", pendingQuestion: { requestId: "q", questions: [] } }),
      { ...ctx, otherDevice: "nexus" },
    );
    expect(s).toMatchObject({ label: "nexus is waiting for an answer", tone: "quiet" });
  });

  it("keeps a foreground subagent's name when a later background agent is running", () => {
    const s = activityStatus(
      entry([
        user(),
        tool("Agent", { subagent_type: "foreman", description: "Phase 1", run_in_background: false }, "fg"),
        tool("Agent", { subagent_type: "scout", description: "look", run_in_background: true }, "bg"),
      ]),
      ctx,
    );
    expect(s?.label).toBe("foreman");
  });

  it("says working while waiting on the first token", () => {
    expect(activityStatus(entry([user()]), ctx)).toMatchObject({ label: "Working…", since: 1000 });
  });

  it("says thinking while the last item is reasoning", () => {
    expect(activityStatus(entry([user(), { kind: "thinking", id: "th", text: "hm" }]), ctx)?.label).toBe(
      "Thinking…",
    );
  });

  it("says writing while the answer streams", () => {
    const items: TranscriptItem[] = [user(), { kind: "assistant", id: "a", text: "Sure", streaming: true }];
    expect(activityStatus(entry(items), ctx)?.label).toBe("Writing…");
  });

  it("names the open tool and its target", () => {
    const s = activityStatus(entry([user(), tool("Read", { file_path: "/repo/src/a.ts" }, "t1")]), ctx);
    expect(s).toMatchObject({ label: "Reading", detail: "a.ts" });
  });

  it("summarizes a running command", () => {
    const s = activityStatus(entry([user(), tool("Bash", { command: "pnpm test" }, "t1")]), ctx);
    expect(s).toMatchObject({ label: "Running", detail: "pnpm test" });
  });

  it("falls back to the tool's display name for unknown tools", () => {
    const s = activityStatus(entry([user(), tool("mcp__linear__get_issue", { id: "X-1" }, "t1")]), {
      ...ctx,
      naming: { mcpServers: ["linear"] },
    });
    expect(s?.label).toBe("Running linear - get_issue (MCP)");
  });

  it("is back to working once the tool finishes", () => {
    const s = activityStatus(entry([user(), tool("Read", { file_path: "/a.ts" }, "t1", { result: ok() })]), ctx);
    expect(s?.label).toBe("Working…");
  });

  it("keeps showing an earlier parallel call that is still open", () => {
    const s = activityStatus(
      entry([
        user(),
        tool("Bash", { command: "cargo build" }, "t1"),
        tool("Read", { file_path: "/a.ts" }, "t2", { result: ok() }),
      ]),
      ctx,
    );
    expect(s).toMatchObject({ label: "Running", detail: "cargo build" });
  });

  it("counts other open parallel calls", () => {
    const s = activityStatus(
      entry([user(), tool("Read", { file_path: "/a.ts" }, "t1"), tool("Read", { file_path: "/b.ts" }, "t2")]),
      ctx,
    );
    expect(s).toMatchObject({ label: "Reading", detail: "b.ts", more: 1 });
  });

  it("skips bookkeeping tools", () => {
    const s = activityStatus(entry([user(), tool("TodoWrite", { todos: [] }, "t1")]), ctx);
    expect(s?.label).toBe("Working…");
  });

  it("does not treat a background spawn as the current step", () => {
    const s = activityStatus(
      entry([user(), tool("Bash", { command: "sleep 99", run_in_background: true }, "t1")]),
      ctx,
    );
    expect(s?.label).toBe("Working…");
  });

  it("ignores open tools from a previous turn", () => {
    const s = activityStatus(entry([tool("Read", { file_path: "/a.ts" }, "t1"), user("u2")]), ctx);
    expect(s?.label).toBe("Working…");
  });

  it("ignores open tools before a relaunch", () => {
    const s = activityStatus(
      entry([user(), tool("Read", { file_path: "/a.ts" }, "t1"), { kind: "relaunched", id: "r" }]),
      ctx,
    );
    expect(s?.label).toBe("Working…");
  });

  it("reports a foreground subagent with its latest step", () => {
    const s = activityStatus(
      entry([
        user(),
        tool("Agent", { subagent_type: "foreman", description: "Phase 1", run_in_background: false }, "ag"),
        tool("Edit", { file_path: "/repo/x.rs" }, "c1", { parentId: "ag", result: ok() }),
      ]),
      ctx,
    );
    expect(s).toMatchObject({ label: "foreman", detail: "Edit x.rs" });
  });

  it("does not let a subagent's own thinking replace the subagent label", () => {
    const s = activityStatus(
      entry([
        user(),
        tool("Agent", { subagent_type: "foreman", run_in_background: false }, "ag"),
        { kind: "thinking", id: "th", text: "…", parentId: "ag" },
      ]),
      ctx,
    );
    expect(s?.label).toBe("foreman");
  });

  it("says how long the last turn took once idle", () => {
    const endedAt = new Date(2026, 9, 27, 10, 20).getTime();
    const s = activityStatus(
      entry([user()], { streaming: false, workingSince: null, lastTurn: { ms: 243_000, endedAt } }),
      ctx,
    );
    expect(s).toMatchObject({ tone: "quiet", since: null });
    expect(s?.label).toBe("Worked for 4m 03s");
    expect(s?.detail).toMatch(/^done 10:20/);
  });

  it("carries the recap written for that turn", () => {
    const lastTurn = { ms: 1000, endedAt: 50 };
    const s = activityStatus(
      entry([user()], { streaming: false, lastTurn, recap: { text: "You asked for X; it's done.", endedAt: 50 } }),
      ctx,
    );
    expect(s?.recap).toBe("You asked for X; it's done.");
  });

  it("drops a recap written for an earlier turn", () => {
    const s = activityStatus(
      entry([user()], {
        streaming: false,
        lastTurn: { ms: 1000, endedAt: 90 },
        recap: { text: "stale", endedAt: 50 },
      }),
      ctx,
    );
    expect(s?.recap).toBeUndefined();
  });

  it("never shows a recap on a live turn", () => {
    const s = activityStatus(entry([user()], { recap: { text: "stale", endedAt: 1 } }), ctx);
    expect(s?.recap).toBeUndefined();
  });

  it("lets a live turn replace the last turn's summary", () => {
    const s = activityStatus(entry([user()], { lastTurn: { ms: 1000, endedAt: 1 } }), ctx);
    expect(s?.label).toBe("Working…");
  });

  it("drops the summary once the session ends", () => {
    const s = activityStatus(
      entry([user()], { state: "completed", streaming: false, lastTurn: { ms: 1000, endedAt: 1 } }),
      ctx,
    );
    expect(s).toBeNull();
  });
});

describe("nextTurn", () => {
  const view = (over: Partial<ActivityInput> = {}) => entry([user()], over);
  const idle = view({ streaming: false });
  const streaming = view();
  const approving = view({ streaming: false, state: "awaiting_permission" });

  it("starts timing when the stream starts", () => {
    expect(nextTurn(idle, streaming, 100)).toEqual({ turnSince: 100, lastTurn: null });
  });

  it("keeps one start across an approval wait", () => {
    const a = { ...streaming, ...nextTurn(idle, streaming, 100) };
    const b = { ...approving, ...nextTurn(a, approving, 200) };
    const c = { ...streaming, ...nextTurn(b, streaming, 300) };
    expect(nextTurn(c, idle, 1100)).toEqual({ turnSince: null, lastTurn: { ms: 1000, endedAt: 1100 } });
  });

  it("does not count spawning the harness as a turn", () => {
    const starting = view({ state: "starting", streaming: false });
    const t = { ...starting, ...nextTurn(undefined, starting, 100) };
    expect(nextTurn(t, idle, 500)).toEqual({ turnSince: null, lastTurn: null });
  });

  it("keeps the last summary while idle", () => {
    const done = { ...idle, lastTurn: { ms: 5, endedAt: 9 } };
    expect(nextTurn(done, idle, 50).lastTurn).toEqual({ ms: 5, endedAt: 9 });
  });

  it("keeps the turn running through a mid-turn reload", () => {
    const a = { ...streaming, ...nextTurn(idle, streaming, 100) };
    const reloading = view({ state: "starting" });
    const b = { ...reloading, ...nextTurn(a, reloading, 200) };
    expect(b).toMatchObject({ turnSince: 100, lastTurn: null });
    expect(nextTurn(b, streaming, 300).turnSince).toBe(100);
  });

  it("records nothing for an entry first seen idle", () => {
    expect(nextTurn(undefined, idle, 50)).toEqual({ turnSince: null, lastTurn: null });
  });
});

describe("recapWanted", () => {
  const last = { ms: 1000, endedAt: 50 };
  const ended = entry([user()], { streaming: false });

  it("wants a recap for a turn that just ended", () => {
    expect(recapWanted(null, last, ended)).toBe(true);
  });

  it("does not ask twice for the same turn", () => {
    expect(recapWanted(last, last, ended)).toBe(false);
  });

  it("skips another device's turn", () => {
    expect(recapWanted(null, last, { ...ended, turnAuthor: "other" })).toBe(false);
  });

  it("skips a session that ended, since the bar never shows its recap", () => {
    expect(recapWanted(null, last, { ...ended, state: "crashed" })).toBe(false);
  });

  it("skips turns a running goal drives", () => {
    const goal = { status: "running" } as ActivityInput["goal"];
    expect(recapWanted(null, last, { ...ended, goal })).toBe(false);
  });
});
