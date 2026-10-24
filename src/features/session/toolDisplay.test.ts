import { describe, expect, it } from "vitest";

import { formatElapsed, formatRelativeTime } from "@/lib/format";
import type { TranscriptItem } from "./transcript";
import {
  baseName,
  detectHandoffDoc,
  docFromTool,
  sessionDocs,
  docTitle,
  editDiffFromTool,
  parseToolInput,
  prettyToolInput,
  resultLine,
  resultPreview,
  toolDisplayName,
  toolFilePath,
  toolSummary,
} from "./toolDisplay";

describe("toolDisplayName", () => {
  const claude = {
    harness: "claude" as const,
    mcpServers: ["claude.ai Atlassian Rovo", "@hypothesi/tauri-mcp-server", "plugin:engineering:linear"],
  };

  it("names a claude MCP tool as claude does, by the server's real name", () => {
    expect(toolDisplayName("mcp__claude_ai_Atlassian_Rovo__getJiraIssue", claude)).toBe(
      "claude.ai Atlassian Rovo - getJiraIssue (MCP)",
    );
    expect(toolDisplayName("mcp___hypothesi_tauri-mcp-server__webview_execute_js", claude)).toBe(
      "@hypothesi/tauri-mcp-server - webview_execute_js (MCP)",
    );
    expect(toolDisplayName("mcp__plugin_engineering_linear__get_issue", claude)).toBe(
      "plugin:engineering:linear - get_issue (MCP)",
    );
  });

  it("falls back to the id's server segment when claude never listed the server", () => {
    expect(toolDisplayName("mcp__ynab__list_budgets", claude)).toBe("ynab - list_budgets (MCP)");
    expect(toolDisplayName("mcp__ynab__list_budgets")).toBe("ynab - list_budgets (MCP)");
  });

  it("names a codex MCP tool as codex does", () => {
    expect(toolDisplayName("mcp__linear__create_issue", { harness: "codex", mcpServers: [] })).toBe(
      "linear.create_issue",
    );
  });

  it("leaves built-in tools and malformed MCP names alone", () => {
    expect(toolDisplayName("Bash", claude)).toBe("Bash");
    expect(toolDisplayName("mcp__orphan", claude)).toBe("mcp__orphan");
    expect(toolDisplayName("mcp____tool", claude)).toBe("mcp____tool");
    expect(toolDisplayName("mcp__server__", claude)).toBe("mcp__server__");
  });
});

describe("parseToolInput", () => {
  it("parses a JSON object", () => {
    expect(parseToolInput('{"a":1}')).toEqual({ a: 1 });
  });

  it("rejects non-objects and junk", () => {
    expect(parseToolInput("[]")).toBeNull();
    expect(parseToolInput('"hi"')).toBeNull();
    expect(parseToolInput("not json")).toBeNull();
    expect(parseToolInput("")).toBeNull();
    expect(parseToolInput("null")).toBeNull();
  });
});

describe("toolSummary", () => {
  it("surfaces the bash command, collapsing to one line", () => {
    expect(toolSummary("Bash", '{"command":"git status"}')).toBe("git status");
    expect(toolSummary("Bash", '{"command":"line one\\nline two"}')).toBe("line one\nline two");
    expect(toolSummary("Bash", '{"command":"one\\ntwo\\nthree"}')).toBe("one\ntwo…");
    const long = "git log && ".repeat(30);
    expect(toolSummary("Bash", JSON.stringify({ command: long }))).toBe(`${long.slice(0, 160).trim()}…`);
  });

  it("surfaces file paths for file tools", () => {
    expect(toolSummary("Read", '{"file_path":"/a/b.ts"}')).toBe("/a/b.ts");
    expect(toolSummary("Edit", '{"file_path":"/a/b.ts","old_string":"x"}')).toBe("/a/b.ts");
    expect(toolSummary("Write", '{"path":"/a/c.md"}')).toBe("/a/c.md");
  });

  it("surfaces search patterns with optional scope", () => {
    expect(toolSummary("Grep", '{"pattern":"foo"}')).toBe("foo");
    expect(toolSummary("Grep", '{"pattern":"foo","path":"src"}')).toBe("foo  in src");
    expect(toolSummary("Glob", '{"pattern":"**/*.ts"}')).toBe("**/*.ts");
  });

  it("summarises task / web / todo tools", () => {
    expect(toolSummary("Task", '{"description":"find bugs","subagent_type":"x"}')).toBe("find bugs");
    expect(toolSummary("Agent", '{"description":"map the store"}')).toBe("map the store");
    expect(toolSummary("WebSearch", '{"query":"tauri channel"}')).toBe("tauri channel");
    expect(toolSummary("TodoWrite", '{"todos":[{},{},{}]}')).toBe("3 items");
  });

  it("surfaces the question text for AskUserQuestion", () => {
    expect(toolSummary("AskUserQuestion", '{"questions":[{"question":"Tabs or spaces?"}]}')).toBe(
      "Tabs or spaces?",
    );
    expect(
      toolSummary("AskUserQuestion", '{"questions":[{"question":"A?"},{"question":"B?"}]}'),
    ).toBe("A? (+1 more)");
  });

  it("names the skill (not its args) for the Skill tool", () => {
    expect(toolSummary("Skill", '{"args":"22","skill":"describe_pr"}')).toBe("describe_pr  22");
    expect(toolSummary("Skill", '{"skill":"describe_pr"}')).toBe("describe_pr");
    expect(toolSummary("Skill", '{"skill":"create_plan","args":"add\\nauth"}')).toBe(
      "create_plan  add …",
    );
  });

  it("reads a Workflow row's label out of the script's meta block", () => {
    const withDescription =
      "export const meta = {\n" +
      "  name: 'find-flaky-tests',\n" +
      "  description: 'Find flaky tests and propose fixes',\n" +
      "  phases: [\n" +
      "    { title: 'Scan', detail: 'grep test logs for retries' },\n" +
      "  ],\n" +
      "}\n" +
      "phase('Scan')\n" +
      "const flaky = await agent('grep CI logs for retries', {schema: FLAKY_SCHEMA})\n";
    expect(toolSummary("Workflow", JSON.stringify({ script: withDescription }))).toBe(
      "Find flaky tests and propose fixes",
    );

    const nameOnly =
      "export const meta = {\n" + "  name: 'audit-deps',\n" + "  phases: [],\n" + "}\n";
    expect(toolSummary("Workflow", JSON.stringify({ script: nameOnly }))).toBe("audit-deps");
  });

  it("lists every arg for unknown and MCP tools, capped at 160 characters", () => {
    expect(toolSummary("Mystery", '{"n":1,"label":"hello"}')).toBe('n: 1, label: "hello"');
    expect(toolSummary("Mystery", '{"text":"a\\nb"}')).toBe('text: "a\\nb"');
    expect(toolSummary("Mystery", "{}")).toBe("");
    const long = "x".repeat(300);
    const summary = toolSummary("mcp__ynab__list_budgets", JSON.stringify({ q: long }));
    expect(summary).toBe(`${`q: "${long}"`.slice(0, 160)}…`);
  });

  it("is empty when input does not parse", () => {
    expect(toolSummary("Bash", "garbage")).toBe("");
  });
});

describe("toolFilePath", () => {
  it("returns the path for file-targeting tools", () => {
    expect(toolFilePath("Read", '{"file_path":"/a/b.ts"}')).toBe("/a/b.ts");
    expect(toolFilePath("Edit", '{"file_path":"/a/b.ts","old_string":"x"}')).toBe("/a/b.ts");
    expect(toolFilePath("Write", '{"path":"/a/c.md"}')).toBe("/a/c.md");
    expect(toolFilePath("NotebookEdit", '{"notebook_path":"/a/n.ipynb"}')).toBe("/a/n.ipynb");
  });

  it("returns null for non-file tools and unparseable input", () => {
    expect(toolFilePath("Bash", '{"command":"ls"}')).toBeNull();
    expect(toolFilePath("Grep", '{"pattern":"foo","path":"src"}')).toBeNull();
    expect(toolFilePath("Read", "garbage")).toBeNull();
    expect(toolFilePath("Read", "{}")).toBeNull();
  });
});

describe("editDiffFromTool", () => {
  it("derives a diff from an Edit's old/new strings", () => {
    const diff = editDiffFromTool(
      "Edit",
      JSON.stringify({ file_path: "/a/b.ts", old_string: "a\nold\nc", new_string: "a\nnew\nc" }),
    );
    expect(diff).toEqual({
      lines: [
        { kind: "context", text: " a" },
        { kind: "del", text: "-old" },
        { kind: "add", text: "+new" },
        { kind: "context", text: " c" },
      ],
      additions: 1,
      deletions: 1,
    });
  });

  it("stacks a MultiEdit's edits with a hunk separator", () => {
    const diff = editDiffFromTool(
      "MultiEdit",
      JSON.stringify({
        file_path: "/a/b.ts",
        edits: [
          { old_string: "x", new_string: "y" },
          { old_string: "p", new_string: "q" },
        ],
      }),
    );
    expect(diff?.additions).toBe(2);
    expect(diff?.deletions).toBe(2);
    expect(diff?.lines.map((l) => l.kind)).toEqual([
      "del",
      "add",
      "hunk",
      "del",
      "add",
    ]);
  });

  it("returns null for non-edit tools and unparseable input", () => {
    expect(editDiffFromTool("Read", '{"file_path":"/a/b.ts"}')).toBeNull();
    expect(editDiffFromTool("Edit", "garbage")).toBeNull();
    expect(editDiffFromTool("MultiEdit", '{"file_path":"/a/b.ts"}')).toBeNull();
  });
});

describe("baseName", () => {
  it("returns the final path segment", () => {
    expect(baseName("/Users/jt/repo/src/main.rs")).toBe("main.rs");
    expect(baseName("file.ts")).toBe("file.ts");
    expect(baseName("/a/b/")).toBe("b");
  });
});

describe("prettyToolInput", () => {
  it("pretty-prints valid JSON", () => {
    expect(prettyToolInput('{"a":1}')).toBe('{\n  "a": 1\n}');
  });

  it("returns the raw string for unparseable input", () => {
    expect(prettyToolInput("not json")).toBe("not json");
    expect(prettyToolInput("")).toBe("");
  });
});

describe("resultPreview", () => {
  it("strips cat -n line numbers and shows the first meaningful line", () => {
    expect(resultPreview("  12\tuse anyhow::Result;", false)).toBe("use anyhow::Result;");
    expect(resultPreview("\n\n   first real line\nsecond", false)).toBe("first real line");
  });

  it("falls back when there is no output", () => {
    expect(resultPreview("", false)).toBe("(no output)");
    expect(resultPreview("   \n  ", true)).toBe("(error)");
  });

  it("leaves ordinary content untouched", () => {
    expect(resultPreview("Done (12 tool uses)", false)).toBe("Done (12 tool uses)");
  });
});

describe("resultLine", () => {
  it("shows a command's first line and counts the rest", () => {
    expect(resultLine("Bash", '{"command":"ls"}', "a\n\nb\nc\n", false)).toEqual({ text: "a", more: 2 });
  });

  it("says a backgrounded command is running rather than echoing the launch notice", () => {
    expect(
      resultLine("Bash", '{"command":"x","run_in_background":true}', "Command running in background with ID: b1", false),
    ).toEqual({ text: "Running in the background", more: 0 });
  });

  it("counts matches for searches", () => {
    expect(resultLine("Glob", '{"pattern":"*.ts"}', "/a.ts\n/b.ts\n(Results are truncated.)", false).text).toBe(
      "Found 2 files",
    );
    expect(resultLine("Glob", '{"pattern":"*.ts"}', "No files found", false).text).toBe("No files found");
    expect(
      resultLine("Grep", '{"pattern":"x","output_mode":"content"}', "a.rs:1:x\nb.rs:4:x", false),
    ).toEqual({ text: "Found 2 lines", more: 0 });
    expect(resultLine("Grep", '{"pattern":"x"}', "Found 3 files\na\nb\nc", false)).toEqual({
      text: "Found 3 files",
      more: 0,
    });
  });

  it("unwraps a tool error", () => {
    expect(resultLine("Read", "{}", "<tool_use_error>File does not exist.</tool_use_error>", true).text).toBe(
      "File does not exist.",
    );
  });
});

describe("docFromTool", () => {
  it("recognises research, plan, and handoff docs written by the agent", () => {
    expect(
      docFromTool(
        "Write",
        '{"file_path":"/Users/jt/thoughts/repos/x/shared/research/2026-06-15-auth.md"}',
        {},
      ),
    ).toEqual({
      path: "/Users/jt/thoughts/repos/x/shared/research/2026-06-15-auth.md",
      kind: "research",
    });
    expect(docFromTool("Edit", '{"file_path":"/t/shared/plans/big-refactor.md"}', {})).toEqual({
      path: "/t/shared/plans/big-refactor.md",
      kind: "plan",
    });
    expect(
      docFromTool("Write", '{"file_path":"/t/shared/handoffs/2026-06-19-context.md"}', {}),
    ).toEqual({
      path: "/t/shared/handoffs/2026-06-19-context.md",
      kind: "handoff",
    });
  });

  it("ignores non-doc writes and non-write tools", () => {
    expect(docFromTool("Write", '{"file_path":"/repo/src/main.rs"}', {})).toBeNull();
    expect(docFromTool("Write", '{"file_path":"/t/shared/research/notes.txt"}', {})).toBeNull();
    expect(docFromTool("Read", '{"file_path":"/t/shared/research/x.md"}', {})).toBeNull();
    expect(docFromTool("Bash", '{"command":"ls"}', {})).toBeNull();
  });

  describe("shell writes", () => {
    const PLAN = "/t/shared/plans/2026-08-16-skill-bar.md";
    const RESEARCH = "/t/shared/research/2026-08-16-auth.md";
    const HANDOFF = "/t/shared/handoffs/ctx.md";
    const written = {
      "2026-08-16-skill-bar.md": PLAN,
      "2026-08-16-auth.md": RESEARCH,
      "ctx.md": HANDOFF,
    };
    const bash = (command: string) => docFromTool("Bash", JSON.stringify({ command }), written);

    it("recognises a doc the index confirms the command wrote", () => {
      expect(bash(`cat > ${PLAN} <<'EOF'\n# Plan\nEOF`)).toEqual({
        path: PLAN,
        kind: "plan",
      });
      expect(bash(`cp "$P" ${RESEARCH}`)).toEqual({
        path: RESEARCH,
        kind: "research",
      });
      expect(bash(`printf '%s' "$BODY" | tee -a '${HANDOFF}'`)).toEqual({
        path: HANDOFF,
        kind: "handoff",
      });
    });

    it("recognises a doc path assigned to a variable a script writes through", () => {
      expect(
        bash(
          `OUT=${PLAN}\npython3 - "$P" "$OUT" <<'PY'\nimport sys\nopen(sys.argv[2],'w').write(open(sys.argv[1]).read())\nPY`,
        ),
      ).toEqual({ path: PLAN, kind: "plan" });
    });

    it("ignores a doc the command only reads, however it redirects", () => {
      const read = (command: string) => docFromTool("Bash", JSON.stringify({ command }), {});
      expect(read("grep -n '^status:' /t/shared/plans/old-plan.md")).toBeNull();
      expect(read("cat /t/shared/research/a.md /t/shared/research/b.md | head -40")).toBeNull();
      expect(read('P=/t/shared/plans/old-plan.md; grep -c . "$P" 2>/dev/null')).toBeNull();
      expect(read("git show HEAD:thoughts/shared/plans/old-plan.md > /tmp/old.txt")).toBeNull();
      expect(read(`cat > /t/shared/plans/never-landed.md <<'MD'\nx\nMD`)).toBeNull();
    });

    it("resolves an unexpanded or relative mention to the indexed absolute path", () => {
      expect(bash(`cat > "$THOUGHTS/shared/plans/2026-08-16-skill-bar.md" <<'MD'\nx\nMD`)).toEqual({
        path: PLAN,
        kind: "plan",
      });
      expect(bash("cat > thoughts/shared/plans/2026-08-16-skill-bar.md")).toEqual({
        path: PLAN,
        kind: "plan",
      });
      expect(bash("cat >> ~/thoughts/shared/handoffs/ctx.md")).toEqual({
        path: HANDOFF,
        kind: "handoff",
      });
    });

    it("ignores a path no doc was ever written to", () => {
      expect(
        bash(`cat > /tmp/notes.txt <<'MD'\nplan => /t/shared/plans/phantom.md\nMD`),
      ).toBeNull();
      expect(bash("cp /t/shared/plans/absent.md /tmp/dir/")).toBeNull();
    });

    it("needs the proof map: a write-position target alone is never a doc", () => {
      expect(docFromTool("Bash", JSON.stringify({ command: `cat > ${PLAN}` }), {})).toBeNull();
    });

    it("ignores a doc a later command only names, in read or non-md-destination position", () => {
      const read = (command: string) => docFromTool("Bash", JSON.stringify({ command }), written);
      expect(read(`cat ${PLAN}`)).toBeNull();
      expect(read(`sed -n '1,50p' ${PLAN}`)).toBeNull();
      expect(read(`grep -n foo ${PLAN}`)).toBeNull();
      expect(read(`cp ${PLAN} /tmp/dir/`)).toBeNull();
      expect(read(`mv ${PLAN} /tmp/`)).toBeNull();
    });

    it("recognises write-position targets from the extended table", () => {
      expect(bash('P=/t/shared/plans/old-plan.md; grep -c . "$P" 2>/dev/null')).toBeNull();
      expect(bash(`printf '%s' "$BODY" | tee -a ${HANDOFF}`)).toEqual({
        path: HANDOFF,
        kind: "handoff",
      });
      expect(bash(`mv /tmp/draft.md ${PLAN}`)).toEqual({ path: PLAN, kind: "plan" });
    });

    it("reads a quoted command name", () => {
      expect(bash(`'cp' /tmp/draft.md ${PLAN}`)).toEqual({ path: PLAN, kind: "plan" });
      expect(bash(`"tee" ${HANDOFF} < /tmp/body`)).toEqual({ path: HANDOFF, kind: "handoff" });
    });

    it("ignores the append forms of an fd redirect", () => {
      expect(bash(`grep -c . ${PLAN} 2>>/t/shared/plans/2026-08-16-skill-bar.md`)).toBeNull();
      expect(bash(`grep -c . ${PLAN} &>>${RESEARCH}`)).toBeNull();
      expect(bash(`echo x >>${PLAN}`)).toEqual({ path: PLAN, kind: "plan" });
    });

    it("strips a heredoc body whatever spells its delimiter", () => {
      for (const delim of ["'DOC-BODY'", "'EOF.MD'", '"END-OF-DOC"', "\\EOF"]) {
        const term = delim.replace(/['"\\]/g, "");
        expect(bash(`cat > /tmp/notes.txt <<${delim}\nplan => ${PLAN}\n${term}`)).toBeNull();
      }
      expect(bash(`cat > ${PLAN} <<'DOC-BODY'\n# Plan\nDOC-BODY`)).toEqual({
        path: PLAN,
        kind: "plan",
      });
    });

    it("requires the assigned variable to reach the script", () => {
      expect(bash(`P=${PLAN}\npython3 - <<'PY'\nprint(1)\nPY`)).toBeNull();
      expect(bash(`P=${PLAN}\npython3 - "$P" <<'PY'\nimport sys\nopen(sys.argv[1],'w')\nPY`)).toEqual(
        { path: PLAN, kind: "plan" },
      );
    });
  });

  describe("symlinked worktree paths", () => {
    const INDEXED = "/Users/jt/thoughts/repos/app/shared/plans/2026-08-19-vm.md";
    const NESTED = "/Users/jt/thoughts/repos/app/shared/handoffs/general/ctx.md";
    const written = { "2026-08-19-vm.md": INDEXED, "ctx.md": NESTED };
    const write = (file_path: string) =>
      docFromTool("Write", JSON.stringify({ file_path }), written);

    it("reports the indexed path a shell mention would resolve to", () => {
      expect(write("/Users/jt/wt/app/branch/thoughts/shared/plans/2026-08-19-vm.md")).toEqual({
        path: INDEXED,
        kind: "plan",
      });
      expect(write("/Users/jt/wt/app/branch/thoughts/shared/handoffs/general/ctx.md")).toEqual({
        path: NESTED,
        kind: "handoff",
      });
    });

    it("keeps the written path when the tail says it is a different doc", () => {
      const personal = "/Users/jt/wt/app/branch/thoughts/jt/plans/2026-08-19-vm.md";
      expect(write(personal)).toEqual({ path: personal, kind: "plan" });
      const repoDoc = "/Users/jt/wt/app/branch/docs/plans/2026-08-19-vm.md";
      expect(write(repoDoc)).toEqual({ path: repoDoc, kind: "plan" });
    });
  });
});

describe("detectHandoffDoc", () => {
  const HANDOFF = "/t/shared/handoffs/2026-08-21-ctx.md";
  const written = { "2026-08-21-ctx.md": HANDOFF };

  function tool(id: string, name: string, input: Record<string, unknown>): TranscriptItem {
    return {
      kind: "tool",
      id,
      toolUseId: id,
      name,
      input: JSON.stringify(input),
      result: null,
    };
  }

  it("returns the handoff the session's own transcript wrote", () => {
    const items = [tool("t1", "Write", { file_path: HANDOFF })];
    expect(detectHandoffDoc(items, written, new Set())).toBe(HANDOFF);
  });

  it("returns null when the session only read the handoff, however it names it", () => {
    const items = [tool("t1", "Bash", { command: `cat ${HANDOFF}` })];
    expect(detectHandoffDoc(items, written, new Set())).toBeNull();
  });

  it("returns null once the user has dismissed it", () => {
    const items = [tool("t1", "Write", { file_path: HANDOFF })];
    expect(detectHandoffDoc(items, written, new Set([HANDOFF]))).toBeNull();
  });

  it("returns the newest write, scanning back to front", () => {
    const older = "/t/shared/handoffs/2026-08-20-old.md";
    const items = [
      tool("t1", "Write", { file_path: older }),
      tool("t2", "Write", { file_path: HANDOFF }),
    ];
    expect(detectHandoffDoc(items, { ...written, "2026-08-20-old.md": older }, new Set())).toBe(
      HANDOFF,
    );
  });
});

describe("docTitle", () => {
  it("strips the date prefix and extension, humanizing the slug", () => {
    expect(docTitle("/t/research/2026-06-15-libhyprlayer-crate-extraction.md")).toBe(
      "libhyprlayer crate extraction",
    );
    expect(docTitle("/t/plans/big_refactor.md")).toBe("big refactor");
  });
});

describe("formatElapsed", () => {
  it("shows tenths of a second under a minute", () => {
    expect(formatElapsed(0)).toBe("0.0s");
    expect(formatElapsed(19_300)).toBe("19.3s");
    expect(formatElapsed(59_900)).toBe("59.9s");
  });

  it("switches to minutes + zero-padded seconds at a minute", () => {
    expect(formatElapsed(60_000)).toBe("1m 00s");
    expect(formatElapsed(83_000)).toBe("1m 23s");
    expect(formatElapsed(605_000)).toBe("10m 05s");
  });

  it("switches to hours + minutes past an hour", () => {
    expect(formatElapsed(3_600_000)).toBe("1h 00m");
    expect(formatElapsed(3_840_000)).toBe("1h 04m");
  });

  it("clamps negative inputs to zero", () => {
    expect(formatElapsed(-500)).toBe("0.0s");
  });
});

describe("formatRelativeTime", () => {
  const NOW = 1_700_000_000_000;
  const ago = (ms: number) => NOW - ms;

  it("collapses anything under 10 seconds to 'just now'", () => {
    expect(formatRelativeTime(ago(0), NOW)).toBe("just now");
    expect(formatRelativeTime(ago(9_000), NOW)).toBe("just now");
  });

  it("scales through second, minute, hour, day, week, month, year buckets", () => {
    expect(formatRelativeTime(ago(45_000), NOW)).toBe("45s ago");
    expect(formatRelativeTime(ago(4 * 60_000), NOW)).toBe("4m ago");
    expect(formatRelativeTime(ago(2 * 3_600_000), NOW)).toBe("2h ago");
    expect(formatRelativeTime(ago(3 * 86_400_000), NOW)).toBe("3d ago");
    expect(formatRelativeTime(ago(14 * 86_400_000), NOW)).toBe("2w ago");
    expect(formatRelativeTime(ago(45 * 86_400_000), NOW)).toBe("1mo ago");
    expect(formatRelativeTime(ago(400 * 86_400_000), NOW)).toBe("1y ago");
  });

  it("falls back to 'just now' for null and future timestamps", () => {
    expect(formatRelativeTime(null, NOW)).toBe("just now");
    expect(formatRelativeTime(NOW + 5_000, NOW)).toBe("just now");
  });
});

describe("sessionDocs", () => {
  const tool = (name: string, input: string, id: string) =>
    ({ kind: "tool", id, name, input, parentId: null }) as unknown as TranscriptItem;

  const items = [
    tool("Write", '{"file_path":"/t/shared/plans/a.md"}', "1"),
    tool("Read", '{"file_path":"/t/shared/plans/b.md"}', "2"),
    tool("Edit", '{"file_path":"/t/shared/research/c.md"}', "3"),
    tool("Write", '{"file_path":"/t/shared/plans/a.md"}', "4"),
  ];

  it("projects every doc-creating tool call in traversal order, duplicates kept", () => {
    expect(sessionDocs(items, {})).toEqual([
      { path: "/t/shared/plans/a.md", kind: "plan" },
      { path: "/t/shared/research/c.md", kind: "research" },
      { path: "/t/shared/plans/a.md", kind: "plan" },
    ]);
  });

  it("returns the same array for the same items, so repeat scans are free", () => {
    const written = {};
    expect(sessionDocs(items, written)).toBe(sessionDocs(items, written));
  });

  it("recomputes when `written` changes, since it resolves shell targets", () => {
    const first = sessionDocs(items, {});
    const second = sessionDocs(items, { "x.md": "/t/shared/plans/x.md" });
    expect(second).not.toBe(first);
    expect(second).toEqual(first);
  });

  it("recomputes for a new items array, so an appended event is picked up", () => {
    const grown = [...items, tool("Write", '{"file_path":"/t/shared/handoffs/d.md"}', "5")];
    expect(sessionDocs(grown, {})).toHaveLength(4);
    expect(sessionDocs(items, {})).toHaveLength(3);
  });
});
