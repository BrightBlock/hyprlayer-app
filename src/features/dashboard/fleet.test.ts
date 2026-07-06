import { describe, expect, it } from "vitest";

import type { WorkstreamView } from "@/lib/bindings";
import type { SessionEntry } from "@/features/session/sessionStore";
import { initialSessionView, type TranscriptItem } from "@/features/session/transcript";
import { foldFleetItem, selectFleetKeys, workingLine, type Picked } from "./fleet";

function entry(over: Partial<SessionEntry> & { workstreamId: string }): SessionEntry {
  return {
    ...initialSessionView("running"),
    stage: "implement",
    autoAllow: [],
    live: true,
    permissionMode: null,
    workingSince: null,
    taskStartedAt: {},
    queued: [],
    ...over,
  };
}

function tool(name: string, input: Record<string, unknown>): TranscriptItem {
  return {
    kind: "tool",
    id: "t1",
    toolUseId: "tu1",
    name,
    input: JSON.stringify(input),
    result: null,
  };
}

function view(id: string): WorkstreamView {
  return {
    item: {
      id,
      projectPath: "/repo",
      title: id,
      slug: id,
      branch: `feature/${id}`,
      worktreePath: `/wt/${id}`,
      baseBranch: "main",
      worktreeMode: "isolated",
      stage: "implement",
      status: "active",
      prNumber: null,
      createdAt: 1,
      updatedAt: 2,
      archivedAt: null,
    },
    git: {
      exists: true,
      branch: `feature/${id}`,
      dirty: 0,
      additions: 0,
      deletions: 0,
      untracked: 0,
      ahead: 0,
      behind: 0,
      aheadOfBase: 0,
      thoughts: "ok",
    },
    seed: null,
  };
}

describe("workingLine", () => {
  it("verbalizes the last tool call", () => {
    const items = [tool("Edit", { file_path: "/a/b/RepoSourcePicker.tsx" })];
    expect(workingLine(items)).toBe("editing RepoSourcePicker.tsx");
  });

  it("passes bash commands through", () => {
    expect(workingLine([tool("Bash", { command: "pnpm test" })])).toBe("pnpm test");
  });

  it("falls back to thinking when no tool has run this turn", () => {
    expect(workingLine([])).toBe("thinking…");
    const items: TranscriptItem[] = [
      tool("Read", { file_path: "/x.ts" }),
      { kind: "user", id: "u1", text: "go", stage: null, attachments: [], createdAt: 1 },
    ];
    expect(workingLine(items)).toBe("thinking…");
  });
});

describe("selectFleetKeys", () => {
  it("picks the most urgent session per workstream and sums queued", () => {
    const sessions: Record<string, SessionEntry> = {
      s1: entry({
        workstreamId: "ws1",
        streaming: true,
        workingSince: 111,
        contextTokens: 91_700,
        model: "claude-opus-4-8",
        items: [tool("Edit", { file_path: "/a/Dashboard.tsx" })],
        queued: [{ text: "next", attachments: [] }],
      }),
      s2: entry({
        workstreamId: "ws1",
        state: "awaiting_permission",
        pendingPermission: { requestId: "r1", toolName: "Bash", input: '{"command":"codesign"}' },
      }),
      s3: entry({ workstreamId: "elsewhere", streaming: true }),
    };
    const keys = selectFleetKeys(sessions, ["ws1", "ws2"]);
    expect(keys).toHaveLength(1);
    const [wsId, activity, sessionId, line, , , , , queued] = keys[0].split("\0");
    expect(wsId).toBe("ws1");
    expect(activity).toBe("needs-you");
    expect(sessionId).toBe("s2");
    expect(line).toBe("Bash · codesign");
    expect(queued).toBe("1");
  });

  it("emits nothing for idle workstreams", () => {
    const sessions: Record<string, SessionEntry> = {
      s1: entry({ workstreamId: "ws1", state: "idle" }),
    };
    expect(selectFleetKeys(sessions, ["ws1"])).toHaveLength(0);
  });

  it("carries the meter fields for a working session", () => {
    const sessions: Record<string, SessionEntry> = {
      s1: entry({
        workstreamId: "ws1",
        streaming: true,
        workingSince: 500,
        contextTokens: 42_000,
        model: "claude-sonnet-5",
        items: [tool("Bash", { command: "cargo build" })],
      }),
    };
    const [key] = selectFleetKeys(sessions, ["ws1"]);
    const [, activity, , line, tokens, model, , since] = key.split("\0");
    expect(activity).toBe("working");
    expect(line).toBe("cargo build");
    expect(tokens).toBe("42000");
    expect(model).toBe("claude-sonnet-5");
    expect(since).toBe("500");
  });
});

describe("foldFleetItem", () => {
  const windows = new Map<string, number>();
  const picked = (over: Partial<Picked>): Picked => ({
    activity: "working",
    sessionId: "s1",
    line: "cargo build",
    tokens: null,
    model: null,
    window: null,
    since: null,
    queued: 0,
    ...over,
  });

  it("returns null when every source is idle", () => {
    expect(foldFleetItem(view("ws1"), undefined, "idle", undefined, windows)).toBeNull();
  });

  it("computes the meter with the 200k fallback window", () => {
    const item = foldFleetItem(
      view("ws1"),
      picked({ tokens: 100_000, model: "claude-sonnet-5", since: 7 }),
      "idle",
      undefined,
      windows,
    );
    expect(item?.meter).toEqual({ tokens: 100_000, fraction: 0.5 });
    expect(item?.since).toBe(7);
  });

  it("uses the 1M window for [1m] models", () => {
    const item = foldFleetItem(
      view("ws1"),
      picked({ tokens: 100_000, model: "claude-opus-4-8[1m]" }),
      "idle",
      undefined,
      windows,
    );
    expect(item?.meter?.fraction).toBe(0.1);
  });

  it("lets a blocked pty agent outrank a store review", () => {
    const item = foldFleetItem(
      view("ws1"),
      picked({ activity: "review" }),
      "needs-you",
      undefined,
      windows,
    );
    expect(item?.activity).toBe("needs-you");
    expect(item?.sessionId).toBeNull(); // pty agents have no session tab to focus
  });

  it("lets a registry needs-you outrank the store, carrying its session", () => {
    const item = foldFleetItem(
      view("ws1"),
      picked({ activity: "review" }),
      "idle",
      { activity: "needs-you", sessionId: "s9", since: 123 },
      windows,
    );
    expect(item?.activity).toBe("needs-you");
    expect(item?.sessionId).toBe("s9");
    expect(item?.since).toBe(123);
  });
});
