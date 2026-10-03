import { describe, expect, it } from "vitest";

import { parsePlanDoc } from "@/features/plan/planDoc";
import type { TranscriptItem } from "@/features/session/transcript";

import { attributeFiles, tasksForFile, type TaskRef } from "./attribution";

type Tool = Extract<TranscriptItem, { kind: "tool" }>;

const WT = "/wt";
const PHASES = parsePlanDoc(
  "## Phase 1: Scaffold\n- [ ] Add the route\n### Success Criteria:\n- [ ] fmt\n\n## Phase 2: Build\n- [ ] Wire the slot\n",
  "/t/plans/p.md",
).phases;

const ok = (content = "ok") => ({ content, isError: false });

function tool(name: string, input: unknown, id: string, result: Tool["result"] = ok(), parentId?: string): Tool {
  return { kind: "tool", id: `i-${id}`, toolUseId: id, name, input: JSON.stringify(input), result, parentId };
}

const todos = (id: string, list: [string, string][], parentId?: string) =>
  tool("TodoWrite", { todos: list.map(([content, status]) => ({ content, status })) }, id, ok(), parentId);
const create = (id: string, subject: string, n: number | null) =>
  tool("TaskCreate", { subject }, id, n === null ? null : ok(`Task #${n} created successfully: ${subject}`));
const update = (id: string, taskId: string, status: string, parentId?: string) =>
  tool("TaskUpdate", { taskId, status }, id, ok(), parentId);
const edit = (id: string, path: string, opts: { result?: Tool["result"]; parentId?: string } = {}) =>
  tool("Edit", { file_path: path, old_string: "a", new_string: "b" }, id, opts.result === undefined ? ok() : opts.result, opts.parentId);

const P1: TaskRef = { phaseLabel: "1", taskKey: null, subject: "Phase 1: Scaffold" };
const P2: TaskRef = { phaseLabel: "2", taskKey: null, subject: "Phase 2: Build" };

function attribute(items: TranscriptItem[], since: string | null = null) {
  return attributeFiles(items, PHASES, WT, since);
}

describe("attributeFiles", () => {
  it("leaves an edit made before any task is in progress unattributed", () => {
    const a = attribute([todos("t1", [["Phase 1: Scaffold", "pending"]]), edit("e1", "/wt/src/a.ts")]);
    expect(a.tracked).toBe(true);
    expect(a.byFile.get("src/a.ts")).toBeUndefined();
  });

  it("gives an edit to the most recently started of two in-progress tasks", () => {
    const a = attribute([
      todos("t1", [["Phase 1: Scaffold", "in_progress"], ["Phase 2: Build", "pending"]]),
      todos("t2", [["Phase 1: Scaffold", "in_progress"], ["Phase 2: Build", "in_progress"]]),
      edit("e1", "/wt/src/a.ts"),
    ]);
    expect(a.byFile.get("src/a.ts")).toEqual([P2]);
  });

  it("follows TodoWrite through reused positional ids", () => {
    const a = attribute([
      todos("t1", [["Phase 1: Scaffold", "in_progress"], ["Phase 2: Build", "pending"]]),
      edit("e1", "/wt/src/a.ts"),
      todos("t2", [["Phase 1: Scaffold", "completed"], ["Phase 2: Build", "in_progress"]]),
      edit("e2", "/wt/src/b.ts"),
      todos("t3", [["Add the route", "in_progress"]]),
      edit("e3", "/wt/src/c.ts"),
    ]);
    expect(a.byFile.get("src/a.ts")).toEqual([P1]);
    expect(a.byFile.get("src/b.ts")).toEqual([P2]);
    expect(a.byFile.get("src/c.ts")).toEqual([{ phaseLabel: "1", taskKey: "P1.1", subject: "Add the route" }]);
  });

  it("gives a subagent's edit to the phase todo in progress", () => {
    const a = attribute([
      todos("t1", [["Phase 1: Scaffold", "in_progress"]]),
      edit("e1", "/wt/src/a.ts", { parentId: "agent-1" }),
    ]);
    expect(a.byFile.get("src/a.ts")).toEqual([P1]);
  });

  it("ignores a subagent's own TodoWrite", () => {
    const a = attribute([
      todos("t1", [["Phase 1: Scaffold", "in_progress"]]),
      todos("t2", [["Wire the slot", "in_progress"]], "agent-1"),
      edit("e1", "/wt/src/a.ts", { parentId: "agent-1" }),
    ]);
    expect(a.byFile.get("src/a.ts")).toEqual([P1]);
  });

  it("lets a subagent's TaskUpdate move the in-progress task, as the tracker does", () => {
    const a = attribute([
      create("c1", "Phase 1: Scaffold", 1),
      create("c2", "Phase 2: Build", 2),
      update("u1", "1", "in_progress"),
      edit("e1", "/wt/src/a.ts"),
      update("u2", "1", "completed", "agent-1"),
      update("u3", "2", "in_progress", "agent-1"),
      edit("e2", "/wt/src/b.ts", { parentId: "agent-1" }),
    ]);
    expect(a.byFile.get("src/a.ts")).toEqual([P1]);
    expect(a.byFile.get("src/b.ts")).toEqual([P2]);
  });

  it("counts only successful edits", () => {
    const a = attribute([
      todos("t1", [["Phase 1: Scaffold", "in_progress"]]),
      edit("e1", "/wt/src/failed.ts", { result: { content: "no match", isError: true } }),
      edit("e2", "/wt/src/running.ts", { result: null }),
    ]);
    expect(a.byFile.size).toBe(0);
  });

  it("drops paths outside the worktree", () => {
    const a = attribute([
      todos("t1", [["Phase 1: Scaffold", "in_progress"]]),
      edit("e1", "/elsewhere/a.ts"),
      edit("e2", "/wtx/a.ts"),
      edit("e3", "/wt/../other/a.ts"),
      edit("e4", "/wt/src/in.ts"),
    ]);
    expect([...a.byFile.keys()]).toEqual(["src/in.ts"]);
  });

  it("keys an edit by the path it was made on, and finds it again after a rename", () => {
    const a = attribute([todos("t1", [["Phase 1: Scaffold", "in_progress"]]), edit("e1", "/wt/src/old.ts")]);
    expect([...a.byFile.keys()]).toEqual(["src/old.ts"]);
    expect(tasksForFile(a, { path: "src/new.ts", oldPath: "src/old.ts" })).toEqual([P1]);
    expect(tasksForFile(a, { path: "src/other.ts", oldPath: null })).toEqual([]);
  });

  it("is untracked without tracking tools, or with only a subagent's TodoWrite", () => {
    expect(attribute([edit("e1", "/wt/src/a.ts")])).toEqual({ byFile: new Map(), tracked: false });
    const sub = attribute([todos("t1", [["Phase 1: Scaffold", "in_progress"]], "agent-1"), edit("e1", "/wt/src/a.ts")]);
    expect(sub).toEqual({ byFile: new Map(), tracked: false });
  });

  it("picks up a result that lands on an earlier item after later items were added", () => {
    const pending = edit("e1", "/wt/src/a.ts", { result: null });
    const items: TranscriptItem[] = [
      todos("t1", [["Phase 1: Scaffold", "in_progress"], ["Phase 2: Build", "pending"]]),
      pending,
      todos("t2", [["Phase 1: Scaffold", "completed"], ["Phase 2: Build", "in_progress"]]),
    ];
    expect(attribute(items).byFile.size).toBe(0);
    const landed = items.slice();
    landed[1] = { ...pending, result: ok() };
    expect(attribute(landed).byFile.get("src/a.ts")).toEqual([P1]);
  });

  it("handles parallel TaskCreate results arriving out of order", () => {
    const first = create("c1", "Phase 1: Scaffold", null);
    const items: TranscriptItem[] = [
      first,
      create("c2", "Phase 2: Build", 2),
      update("u1", "2", "in_progress"),
      edit("e1", "/wt/src/b.ts"),
    ];
    expect(attribute(items).byFile.get("src/b.ts")).toEqual([P2]);
    const landed: TranscriptItem[] = [
      { ...first, result: ok("Task #1 created successfully: Phase 1: Scaffold") },
      ...items.slice(1),
      update("u2", "2", "completed"),
      update("u3", "1", "in_progress"),
      edit("e2", "/wt/src/a.ts"),
    ];
    const a = attribute(landed);
    expect(a.byFile.get("src/b.ts")).toEqual([P2]);
    expect(a.byFile.get("src/a.ts")).toEqual([P1]);
  });

  it("prefers a matched phase todo over a newer unmatched in-progress one", () => {
    const a = attribute([
      todos("t1", [["Phase 1: Scaffold", "in_progress"]]),
      todos("t2", [["Phase 1: Scaffold", "in_progress"], ["Regenerate bindings", "in_progress"]]),
      edit("e1", "/wt/src/a.ts"),
      todos("t3", [["Phase 1: Scaffold", "completed"], ["Regenerate bindings", "in_progress"]]),
      edit("e2", "/wt/src/b.ts"),
    ]);
    expect(a.byFile.get("src/a.ts")).toEqual([P1]);
    expect(a.byFile.get("src/b.ts")).toEqual([{ phaseLabel: null, taskKey: null, subject: "Regenerate bindings" }]);
  });

  it("never lets a task that predates the plan claim an edit", () => {
    const a = attribute(
      [
        create("c1", "Phase 2: Build", 1),
        update("u1", "1", "in_progress"),
        tool("ExitPlanMode", { plan: "# Plan" }, "plan"),
        edit("e1", "/wt/src/a.ts"),
        create("c2", "Phase 1: Scaffold", 2),
        update("u2", "2", "in_progress"),
        edit("e2", "/wt/src/b.ts"),
      ],
      "plan",
    );
    expect(a.byFile.get("src/a.ts")).toBeUndefined();
    expect(a.byFile.get("src/b.ts")).toEqual([P1]);
  });

  it("attributes every file of a Codex multi-file MultiEdit", () => {
    const a = attribute([
      todos("t1", [["Phase 2: Build", "in_progress"]]),
      tool(
        "MultiEdit",
        { file_path: "/wt/src/a.ts", edits: [{ file_path: "/wt/src/a.ts" }, { file_path: "/wt/src/b.ts" }, { file_path: 7 }] },
        "m1",
      ),
    ]);
    expect([...a.byFile.keys()].sort()).toEqual(["src/a.ts", "src/b.ts"]);
    expect(a.byFile.get("src/b.ts")).toEqual([P2]);
  });

  it("leaves a file changed only by a Bash command in Other changes", () => {
    const a = attribute([
      todos("t1", [["Phase 1: Scaffold", "in_progress"]]),
      tool("Bash", { command: "cargo test" }, "b1", ok("wrote src/lib/bindings.ts")),
      tool("Read", { file_path: "/wt/src/read.ts" }, "r1"),
    ]);
    expect(a.byFile.size).toBe(0);
  });

  it("counts Write and NotebookEdit, and maps a check-row todo to its phase", () => {
    const a = attribute([
      todos("t1", [["fmt", "in_progress"]]),
      tool("Write", { file_path: "/wt/src/w.ts", content: "" }, "w1"),
      tool("NotebookEdit", { notebook_path: "/wt/nb.ipynb", new_source: "" }, "n1"),
    ]);
    const ref = { phaseLabel: "1", taskKey: null, subject: "fmt" };
    expect(a.byFile.get("src/w.ts")).toEqual([ref]);
    expect(a.byFile.get("nb.ipynb")).toEqual([ref]);
  });

  it("lists every task that touched a file, in first-touch order", () => {
    const a = attribute([
      todos("t1", [["Phase 1: Scaffold", "in_progress"], ["Phase 2: Build", "pending"]]),
      edit("e1", "/wt/src/a.ts"),
      todos("t2", [["Phase 1: Scaffold", "completed"], ["Phase 2: Build", "in_progress"]]),
      edit("e2", "/wt/src/a.ts"),
      todos("t3", [["Phase 1: Scaffold", "in_progress"], ["Phase 2: Build", "completed"]]),
      edit("e3", "/wt/src/a.ts"),
    ]);
    expect(a.byFile.get("src/a.ts")).toEqual([P1, P2]);
  });
});
