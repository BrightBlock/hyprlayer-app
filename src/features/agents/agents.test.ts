import { describe, expect, it } from "vitest";

import type { AgentFile, AgentLibrarySettings, AgentRule, AgentSkill } from "@/lib/bindings";
import {
  AGENT_TOOLS,
  agentRows,
  collaboratorFor,
  DEFAULT_OVERLAY,
  dotColor,
  duplicateAgent,
  editorTabs,
  extraSkills,
  extraTools,
  initialsOf,
  inheritsAllTools,
  isAllProjects,
  isSystem,
  isUnsaved,
  mcpEnabled,
  mcpTools,
  modelLabel,
  newAgent,
  newRule,
  overlayFor,
  promptStats,
  readOnlyTools,
  hasUnknownMemoryScope,
  memoryLabel,
  MEMORY_SCOPES,
  setMcp,
  setSkill,
  setTool,
  shortDescription,
  skillEnabled,
  skillsRestricted,
  skillSummary,
  statusOf,
  syncSkillRules,
  toolEnabled,
  toolSummary,
  visibleAgents,
} from "./agents";

const agent = (over: Partial<AgentFile>): AgentFile => ({
  path: `/home/u/.claude/agents/${over.name ?? "a"}.md`,
  source: "user",
  plugin: null,
  name: "a",
  description: "",
  tools: [],
  skills: [],
  model: null,
  color: null,
  memory: null,
  prompt: "",
  updatedAt: null,
  shippedPath: null,
  modified: false,
  ...over,
});

const library = (over: Partial<AgentLibrarySettings> = {}): AgentLibrarySettings => ({
  folders: [],
  overlays: {},
  defaultAgent: null,
  ...over,
});

const skill = (over: Partial<AgentSkill> = {}): AgentSkill => ({
  name: "research-codebase",
  description: "",
  source: "project",
  plugin: null,
  chars: 3800,
  userOnly: false,
  ...over,
});

describe("newAgent", () => {
  it("starts unsaved, inheriting model and tools", () => {
    const a = newAgent("user");
    expect(isUnsaved(a)).toBe(true);
    expect(a.path).toBe("");
    expect(a.model).toBeNull();
    expect(inheritsAllTools(a)).toBe(true);
  });

  it("is saved once it has a path", () => {
    expect(isUnsaved(agent({ name: "x" }))).toBe(false);
  });
});

describe("tools", () => {
  it("treats an empty list as inheriting, not as no tools", () => {
    expect(inheritsAllTools(agent({ tools: [] }))).toBe(true);
    expect(toolEnabled(agent({ tools: [] }), "Bash")).toBe(true);
    expect(toolSummary(agent({ tools: [] }))).toMatch(/inherits every tool/i);
  });

  it("spells the rest out when the first tool is turned off", () => {
    // The file has no way to say "all but one", so the toggle materializes the list.
    const next = setTool(agent({ tools: [] }), "Bash", false);
    expect(next).toHaveLength(AGENT_TOOLS.length - 1);
    expect(next).not.toContain("Bash");
  });

  it("keeps entries it doesn't model when materializing", () => {
    const a = agent({ tools: ["Read", "mcp__notion__*", "SomeNewTool"] });
    expect(setTool(a, "Read", false)).toEqual(["mcp__notion__*", "SomeNewTool"]);
    expect(readOnlyTools(a)).toEqual(["Read", "Glob", "Grep", "mcp__notion__*", "SomeNewTool"]);
  });

  it("reports unmodelled non-MCP entries so a save can't look lossy", () => {
    expect(extraTools(["Read", "mcp__notion__*", "SomeNewTool"])).toEqual(["SomeNewTool"]);
  });

  it("counts only the tools the editor models", () => {
    expect(toolSummary(agent({ tools: ["Read", "Grep", "mcp__x__y"] }))).toMatch(
      new RegExp(`^2 of ${AGENT_TOOLS.length} enabled`),
    );
  });
});

describe("mcp", () => {
  it("reads a server's state off the agent's own tool list", () => {
    const a = agent({ tools: ["Read", "mcp__github__create_pr"] });
    expect(mcpEnabled(a, "github")).toBe(true);
    expect(mcpEnabled(a, "linear")).toBe(false);
    expect(mcpTools(a, "github")).toEqual(["mcp__github__create_pr"]);
  });

  it("enables a server wholesale and removes every trace when turned off", () => {
    const a = agent({ tools: ["Read"] });
    const on = setMcp(a, "github", true);
    expect(on).toEqual(["Read", "mcp__github__*"]);
    expect(setMcp({ ...a, tools: on }, "github", false)).toEqual(["Read"]);
  });
});

describe("skills", () => {
  it("treats absence from the list as off", () => {
    expect(skillEnabled(agent({ skills: [] }), "research-codebase")).toBe(false);
    expect(skillEnabled(agent({ skills: ["research-codebase"] }), "research-codebase")).toBe(true);
  });

  it("adds bare Skill once — never Skill(<name>) — to a pinned tool list", () => {
    const a = agent({ tools: ["Read"], skills: [] });
    const on = setSkill(a, "research-codebase", true);
    expect(on.skills).toEqual(["research-codebase"]);
    expect(on.tools).toEqual(["Read", "Skill"]);

    const second = setSkill({ ...a, tools: on.tools, skills: on.skills }, "create-plan", true);
    expect(second.tools.filter((t) => t === "Skill")).toHaveLength(1);
  });

  it("leaves Skill in place when the last skill is turned off — an explicit list is the user's", () => {
    const a = agent({ tools: ["Read", "Skill"], skills: ["research-codebase"] });
    const off = setSkill(a, "research-codebase", false);
    expect(off.skills).toEqual([]);
    expect(off.tools).toContain("Skill");
  });

  it("leaves an inherit-all agent's tools empty", () => {
    const a = agent({ tools: [], skills: [] });
    expect(setSkill(a, "research-codebase", true).tools).toEqual([]);
  });

  it("reports names discovery didn't match", () => {
    expect(extraSkills(["a", "b"], [skill({ name: "a" })])).toEqual(["b"]);
  });

  it("summarizes the preloaded count and its token cost", () => {
    const a = agent({ skills: ["a"] });
    const known = [skill({ name: "a", chars: 3800 }), skill({ name: "b" })];
    expect(skillSummary(a, known)).toBe("1 of 2 preloaded · ~1.0k tokens added per spawn");
  });
});

describe("syncSkillRules", () => {
  const rule = (over: Partial<AgentRule>): AgentRule => ({
    id: "r1",
    action: "allow",
    pattern: "Bash",
    ...over,
  });

  it("replaces a previous Skill(...) block, keeping other rules and their order", () => {
    const rules = [
      rule({ id: "r1", pattern: "Bash(git diff:*)" }),
      rule({ id: "r2", pattern: "Skill(old-skill)" }),
      rule({ id: "r3", action: "deny", pattern: "Skill" }),
      rule({ id: "r4", action: "deny", pattern: "Bash(git push:*)" }),
    ];
    const next = syncSkillRules(rules, ["a", "b"], false);
    expect(next.map((r) => r.pattern)).toEqual([
      "Bash(git diff:*)",
      "Bash(git push:*)",
      "Skill(a)",
      "Skill(b)",
    ]);
  });

  it("appends a trailing deny only when restricted, round-tripping through skillsRestricted", () => {
    const restricted = syncSkillRules([], ["a"], true);
    expect(restricted[restricted.length - 1]).toMatchObject({ action: "deny", pattern: "Skill" });
    expect(skillsRestricted(restricted)).toBe(true);

    const open = syncSkillRules([], ["a"], false);
    expect(skillsRestricted(open)).toBe(false);
  });

  it("numbers new ids past the highest surviving one", () => {
    const rules = [rule({ id: "r5" })];
    expect(syncSkillRules(rules, ["a"], true).map((r) => r.id)).toEqual(["r5", "r6", "r7"]);
  });
});

describe("memory scope", () => {
  it("calls an absent scope Off rather than inventing one", () => {
    expect(memoryLabel(null)).toBe("Off");
    expect(hasUnknownMemoryScope(null)).toBe(false);
  });

  it("labels every scope the editor offers", () => {
    for (const s of MEMORY_SCOPES) {
      expect(memoryLabel(s.key)).toBe(s.label);
      expect(hasUnknownMemoryScope(s.key)).toBe(false);
    }
  });

  it("passes an unrecognized scope through instead of hiding it", () => {
    expect(memoryLabel("some-future-scope")).toBe("some-future-scope");
    expect(hasUnknownMemoryScope("some-future-scope")).toBe(true);
  });
});

describe("shortDescription", () => {
  it("keeps a short description whole", () => {
    expect(shortDescription("Finds where code lives")).toBe("Finds where code lives");
  });

  it("collapses the newlines a frontmatter block scalar leaves behind", () => {
    expect(shortDescription("  Finds where\n  code lives  ")).toBe("Finds where code lives");
  });

  it("stops at the first sentence of a paragraph", () => {
    expect(
      shortDescription("Locates files and directories. Call it with a human language prompt."),
    ).toBe("Locates files and directories.");
  });

  it("elides a first sentence that is itself a paragraph, on a word boundary", () => {
    const out = shortDescription(
      "Use this agent when the user asks questions about Claude Code, the Agent SDK, " +
        "the Claude API, or Claude in Slack, and wants a definitive answer.",
    );
    expect(out).toBe(
      "Use this agent when the user asks questions about Claude Code, the Agent SDK, the Claude…",
    );
    expect(out.length).toBeLessThanOrEqual(91);
  });

  it("is empty for an agent with no description", () => {
    expect(shortDescription("")).toBe("");
  });
});

describe("visibleAgents", () => {
  const all = [
    agent({ name: "codebase-locator", description: "Finds where code lives" }),
    agent({ name: "repo-linter", source: "project", path: "/repo/.claude/agents/repo-linter.md" }),
    agent({ name: "code-reviewer", source: "plugin", plugin: "pr-review-toolkit" }),
  ];

  it("splits your agents from the plugins' system agents", () => {
    expect(
      visibleAgents(all, { tab: "yours", scope: "global", search: "" }).map((a) => a.name),
    ).toEqual(["codebase-locator", "repo-linter"]);
    expect(
      visibleAgents(all, { tab: "system", scope: "global", search: "" }).map((a) => a.name),
    ).toEqual(["code-reviewer"]);
  });

  it("narrows to the project's own agents", () => {
    expect(
      visibleAgents(all, { tab: "yours", scope: "project", search: "" }).map((a) => a.name),
    ).toEqual(["repo-linter"]);
  });

  it("matches name and description case-insensitively", () => {
    expect(
      visibleAgents(all, { tab: "yours", scope: "global", search: "LOCATOR" }).map((a) => a.name),
    ).toEqual(["codebase-locator"]);
    expect(
      visibleAgents(all, { tab: "yours", scope: "global", search: "Where Code" }).map((a) => a.name),
    ).toEqual(["codebase-locator"]);
    expect(visibleAgents(all, { tab: "yours", scope: "global", search: "nope" })).toHaveLength(0);
  });
});

describe("agentRows", () => {
  const loose = agent({ name: "loose" });
  const filed = agent({ name: "filed" });
  const lib = library({
    folders: ["Review"],
    overlays: { [filed.path]: { folder: "Review" } },
  });

  it("puts loose agents first, then each folder's header", () => {
    expect(agentRows([loose, filed], lib, ["Review"], new Set(["Review"]))).toEqual([
      { kind: "agent", agent: loose, indent: 0 },
      { kind: "group", folder: "Review", count: 1, open: true },
      { kind: "agent", agent: filed, indent: 14 },
    ]);
  });

  it("keeps a collapsed folder's header but hides its agents", () => {
    const rows = agentRows([loose, filed], lib, ["Review"], new Set());
    expect(rows).toHaveLength(2);
    expect(rows[1]).toEqual({ kind: "group", folder: "Review", count: 1, open: false });
  });

  it("drops a folder nothing is in rather than rendering a dead header", () => {
    expect(agentRows([loose], lib, ["Review"], new Set(["Review"]))).toHaveLength(1);
  });
});

describe("overlays", () => {
  it("fills every field a stored partial leaves out", () => {
    const lib = library({ overlays: { "/a.md": { effort: "max" } } });
    expect(overlayFor(lib, "/a.md")).toEqual({ ...DEFAULT_OVERLAY, effort: "max" });
    expect(overlayFor(lib, "/missing.md")).toEqual(DEFAULT_OVERLAY);
  });

  it("appends rules with an id that doesn't collide", () => {
    expect(newRule([]).id).toBe("r1");
    expect(newRule([{ id: "r1" }, { id: "r7" }]).id).toBe("r8");
  });
});

describe("status and scope", () => {
  it("calls a plugin agent active and yours a draft until published", () => {
    const plugin = agent({ source: "plugin", plugin: "pr-review-toolkit" });
    expect(statusOf(plugin, DEFAULT_OVERLAY)).toEqual({ label: "Active", tone: "ok" });
    expect(statusOf(agent({}), { ...DEFAULT_OVERLAY, status: "draft" })).toEqual({
      label: "Draft",
      tone: "warn",
    });
    expect(isSystem(plugin)).toBe(true);
  });

  it("reads scope off the directory that holds the file", () => {
    expect(isAllProjects(agent({}))).toBe(true);
    expect(isAllProjects(agent({ source: "project" }))).toBe(false);
  });

  it("dots a draft amber and otherwise follows the frontmatter colour", () => {
    expect(dotColor(agent({}), { ...DEFAULT_OVERLAY, status: "draft" })).toBe("var(--color-warn)");
    expect(dotColor(agent({ color: "Green" }), DEFAULT_OVERLAY)).toBe("var(--color-ok)");
  });
});

describe("editorTabs", () => {
  it("hides Collaborators until there's an account behind it", () => {
    expect(editorTabs(false).map((t) => t.key)).not.toContain("collaborators");
    expect(editorTabs(true).map((t) => t.key)).toContain("collaborators");
  });
});

describe("duplicateAgent", () => {
  it("copies under a new name and no path, so it saves as a new file", () => {
    const source = agent({ name: "codebase-locator", tools: ["Read", "Grep"] });
    const copy = duplicateAgent(source);
    expect(copy.name).toBe("codebase-locator copy");
    expect(copy.path).toBe("");
    // Deep copy: editing the copy's tools must not reach back into the source.
    copy.tools.push("Bash");
    expect(source.tools).toEqual(["Read", "Grep"]);
  });

  it("lands a plugin agent's copy in your own library", () => {
    const copy = duplicateAgent(agent({ source: "plugin", plugin: "pr-review-toolkit" }));
    expect(copy.source).toBe("user");
    expect(copy.plugin).toBeNull();
  });
});

describe("labels", () => {
  it("names a known model and passes an unknown id through", () => {
    expect(modelLabel(null)).toBe("Inherit");
    expect(modelLabel("sonnet")).toBe("Sonnet 5");
    expect(modelLabel("opus")).toBe("Opus 5 (1M)");
    expect(modelLabel("claude-something-new")).toBe("claude-something-new");
  });

  it("never reports zero tokens for a non-empty prompt", () => {
    expect(promptStats("")).toBe("0 chars · ~1 tokens");
    expect(promptStats("x".repeat(38))).toBe("38 chars · ~10 tokens");
  });

  it("derives a person from an invited address", () => {
    expect(collaboratorFor("maya.ortiz@brightblock.ai")).toEqual({
      name: "maya ortiz",
      email: "maya.ortiz@brightblock.ai",
      role: "Editor",
    });
    expect(initialsOf("Maya Ortiz")).toBe("MO");
  });
});
