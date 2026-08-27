import { describe, expect, it } from "vitest";

import type {
  AgentBundleStatus,
  AgentFile,
  AgentLibrarySettings,
  AgentRule,
  AgentSkill,
} from "@/lib/bindings";
import {
  AGENT_PHASE,
  ANY_STAGE,
  PHASE_COLOR,
  SDLC_PHASES,
  STAGE_PHASE,
  STAGES,
} from "@/lib/domain/stages";
import {
  AGENT_TOOLS,
  agentRows,
  bundleChipState,
  collaboratorFor,
  DEFAULT_OVERLAY,
  dotColor,
  duplicateAgent,
  editorTabs,
  extraSkills,
  extraTools,
  folderColor,
  folderList,
  folderOf,
  folderResolver,
  initialsOf,
  inheritsAllTools,
  isAllProjects,
  isManaged,
  isSystem,
  isUnsaved,
  mcpEnabled,
  mcpTools,
  modelLabel,
  newAgent,
  newRule,
  overlayFor,
  phaseFolder,
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
  managed: false,
  name: "a",
  description: "",
  tools: [],
  skills: [],
  model: null,
  color: null,
  memory: null,
  category: null,
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

  it("adds bare Skill once, never Skill(<name>), to a pinned tool list", () => {
    const a = agent({ tools: ["Read"], skills: [] });
    const on = setSkill(a, "research-codebase", true);
    expect(on.skills).toEqual(["research-codebase"]);
    expect(on.tools).toEqual(["Read", "Skill"]);

    const second = setSkill({ ...a, tools: on.tools, skills: on.skills }, "create-plan", true);
    expect(second.tools.filter((t) => t === "Skill")).toHaveLength(1);
  });

  it("leaves Skill in place when the last skill is turned off, an explicit list is the user's", () => {
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
    agent({ name: "cartographer", managed: true }),
  ];

  it("splits your agents from the plugins' system agents", () => {
    expect(
      visibleAgents(all, { tab: "yours", scope: "global", search: "" }).map((a) => a.name),
    ).toEqual(["codebase-locator", "repo-linter"]);
    expect(
      visibleAgents(all, { tab: "system", scope: "global", search: "" }).map((a) => a.name),
    ).toEqual(["code-reviewer", "cartographer"]);
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

const MUTED = "color-mix(in oklab, var(--color-ink-dim) 55%, transparent)";

describe("agentRows", () => {
  const loose = agent({ name: "loose" });
  const filed = agent({ name: "filed" });
  const lib = library({
    folders: ["Review"],
    overlays: { [filed.path]: { folder: "Review" } },
  });

  const folderOf = folderResolver(lib, false);
  const rowsOf = (agents: AgentFile[], closedFolders: ReadonlySet<string>) =>
    agentRows({ agents, folders: ["Review"], closedFolders, folderOf });

  it("puts loose agents first, then each folder's header", () => {
    expect(rowsOf([loose, filed], new Set())).toEqual([
      { kind: "agent", agent: loose, indent: 0 },
      { kind: "group", folder: "Review", count: 1, open: true },
      { kind: "agent", agent: filed, indent: 14 },
    ]);
  });

  it("keeps a collapsed folder's header but hides its agents", () => {
    const rows = rowsOf([loose, filed], new Set(["Review"]));
    expect(rows).toHaveLength(2);
    expect(rows[1]).toEqual({ kind: "group", folder: "Review", count: 1, open: false });
  });

  it("drops a folder nothing is in rather than rendering a dead header", () => {
    expect(rowsOf([loose], new Set())).toHaveLength(1);
  });

  it("keeps an empty folder that keepEmpty names, so a just-created one is visible", () => {
    const rows = agentRows({
      agents: [loose],
      folders: ["Review"],
      closedFolders: new Set(),
      folderOf,
      keepEmpty: new Set(["Review"]),
    });
    expect(rows).toEqual([
      { kind: "agent", agent: loose, indent: 0 },
      { kind: "group", folder: "Review", count: 0, open: true },
    ]);
  });

  it("still drops an empty folder keepEmpty does not name", () => {
    const rows = agentRows({
      agents: [loose],
      folders: ["Review"],
      closedFolders: new Set(),
      folderOf,
      keepEmpty: new Set(["Something else"]),
    });
    expect(rows.some((r) => r.kind === "group")).toBe(false);
  });

  it("renders a folder absent from closedFolders open, including one that appears later", () => {
    const rows = rowsOf([loose, filed], new Set());
    const group = rows.find((r) => r.kind === "group");
    expect(group).toEqual({ kind: "group", folder: "Review", count: 1, open: true });
  });

  it("keeps every member of a folder, not just the first", () => {
    const second = agent({ name: "filed-2", path: "/a/filed-2.md" });
    const many = library({
      folders: ["Review"],
      overlays: { [filed.path]: { folder: "Review" }, [second.path]: { folder: "Review" } },
    });
    const rows = agentRows({
      agents: [filed, second],
      folders: ["Review"],
      closedFolders: new Set(),
      folderOf: folderResolver(many, false),
    });
    expect(rows[0]).toEqual({ kind: "group", folder: "Review", count: 2, open: true });
    expect(rows.filter((r) => r.kind === "agent")).toHaveLength(2);
  });
});

describe("phaseFolder", () => {
  it("maps each of the shipped description phrases to its phase", () => {
    expect(
      phaseFolder(agent({ description: "Spawn during the Plan stage, after drafting" })),
    ).toBe("Design");
    expect(
      phaseFolder(agent({ description: "Spawn one per area during the Research stage" })),
    ).toBe("Discover");
    expect(phaseFolder(agent({ description: "Spawn during the Validate stage" }))).toBe("Verify");
    expect(phaseFolder(agent({ description: "Spawn during the Revise stage" }))).toBe("Verify");
  });

  it("does not match a stage word outside the naming phrase", () => {
    expect(phaseFolder(agent({ description: "Drafts an implementation plan body" }))).toBe(
      ANY_STAGE,
    );
  });

  it("falls to Any stage when the description names no stage", () => {
    expect(phaseFolder(agent({ description: "Finds files matching a pattern." }))).toBe(
      ANY_STAGE,
    );
  });

  it("covers every stage STAGES names, so a new stage can't silently yield undefined", () => {
    for (const { key } of STAGES) {
      expect(STAGE_PHASE[key]).toBeTruthy();
    }
  });

  it("files an agent whose description names no stage by AGENT_PHASE's name entry", () => {
    const expected: Record<string, string> = {
      "adversarial-reviewer": "Verify",
      ship: "Deliver",
      archivist: "Discover",
      "codebase-analyzer": "Discover",
      "codebase-locator": "Discover",
      "codebase-pattern-finder": "Discover",
      "jira-searcher": "Discover",
      "jira-ticket-reader": "Discover",
      "thoughts-analyzer": "Discover",
      "thoughts-locator": "Discover",
      "web-search-researcher": "Discover",
    };
    for (const [name, phase] of Object.entries(expected)) {
      expect(phaseFolder(agent({ name, description: "Names no stage." }))).toBe(phase);
      expect(AGENT_PHASE[name]).toBe(phase);
    }
  });

  it("lets a description that names a stage beat the table", () => {
    expect(
      phaseFolder(agent({ name: "archivist", description: "Spawn during the Ship stage" })),
    ).toBe("Deliver");
    expect(
      phaseFolder(agent({ name: "ship", description: "Spawn during the Research stage" })),
    ).toBe("Discover");
  });

  it("still returns Any stage for an agent in neither the phrase nor the table", () => {
    expect(phaseFolder(agent({ name: "my-helper", description: "Does a thing." }))).toBe(
      ANY_STAGE,
    );
  });

  it("never resolves a name through Object.prototype", () => {
    for (const name of ["toString", "constructor", "hasOwnProperty", "__proto__", "valueOf"]) {
      expect(phaseFolder(agent({ name, description: "Names no stage." }))).toBe(ANY_STAGE);
    }
  });
});

describe("folderOf", () => {
  it("prefers category over a legacy overlay.folder", () => {
    const a = agent({ category: "Spike" });
    expect(folderOf(a, { ...DEFAULT_OVERLAY, folder: "Old" }, true)).toBe("Spike");
  });

  it("falls back to a plugin agent's plugin name when it has no overlay folder", () => {
    const a = agent({ source: "plugin", plugin: "pr-review-toolkit" });
    expect(folderOf(a, DEFAULT_OVERLAY, true)).toBe("pr-review-toolkit");
  });

  it("a plugin agent's own overlay.folder still wins over its plugin name", () => {
    const a = agent({ source: "plugin", plugin: "pr-review-toolkit" });
    expect(folderOf(a, { ...DEFAULT_OVERLAY, folder: "Moved" }, true)).toBe("Moved");
  });

  it("leaves an uncategorized owned agent loose when groupUncategorized is off", () => {
    const a = agent({ description: "Spawn during the Plan stage" });
    expect(folderOf(a, DEFAULT_OVERLAY, false)).toBe("");
    expect(folderOf(a, DEFAULT_OVERLAY, true)).toBe("Design");
  });

  it("a dragged overlay.folder wins over the AGENT_PHASE table", () => {
    const a = agent({ name: "codebase-locator", description: "Names no stage." });
    expect(folderOf(a, DEFAULT_OVERLAY, true)).toBe("Discover");
    expect(folderOf(a, { ...DEFAULT_OVERLAY, folder: "Scratch" }, true)).toBe("Scratch");
  });

  it("a legacy overlay folder still wins over a derived phase, so clearing it must be deliberate", () => {
    const a = agent({ description: "Spawn during the Plan stage" });
    expect(folderOf(a, { ...DEFAULT_OVERLAY, folder: "Old" }, true)).toBe("Old");
    expect(folderOf({ ...a, category: null }, { ...DEFAULT_OVERLAY, folder: "" }, true)).toBe(
      "Design",
    );
  });
});

describe("folderList", () => {
  it("orders phases in SDLC_PHASES order, non-phase folders alphabetically after them, and omits unoccupied phases", () => {
    const research = agent({ name: "r", description: "Spawn during the Research stage" });
    const plan = agent({ name: "p", description: "Spawn during the Plan stage" });
    const spiked = agent({ name: "s", category: "Spike" });
    const lib = library({});
    const list = folderList([research, plan, spiked], lib, true);
    expect(list).toEqual(["Discover", "Design", "Spike"]);
  });

  it("keeps a persisted folder that is named like a phase", () => {
    expect(folderList([], library({ folders: ["Build"] }), true)).toEqual(["Build"]);
  });

  it("lists a persisted folder once even when agents are filed in it", () => {
    const built = agent({ name: "b", category: "Build" });
    expect(folderList([built], library({ folders: ["Build"] }), true)).toEqual(["Build"]);
  });

  it("lists a folder once when a plugin agent is moved into a user-created one", () => {
    const plugin = agent({ path: "/p/a.md", source: "plugin", plugin: "pr-toolkit" });
    const lib = library({ folders: ["Review"], overlays: { "/p/a.md": { folder: "Review" } } });
    expect(folderList([plugin], lib, true)).toEqual(["Review"]);
    const rows = agentRows({
      agents: [plugin],
      folders: folderList([plugin], lib, true),
      closedFolders: new Set(),
      folderOf: folderResolver(lib, true),
    });
    expect(rows.filter((r) => r.kind === "group")).toHaveLength(1);
    expect(rows.filter((r) => r.kind === "agent")).toHaveLength(1);
  });

  it("does not list a folder for agents left loose by the grouping switch", () => {
    const plan = agent({ name: "p", description: "Spawn during the Plan stage" });
    expect(folderList([plan], library({}), false)).toEqual([]);
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

  it("classifies a linked Hyprlayer agent as managed system content", () => {
    const managed = agent({ managed: true });
    expect(isManaged(managed)).toBe(true);
    expect(isSystem(managed)).toBe(true);
    expect(statusOf(managed, { ...DEFAULT_OVERLAY, status: "draft" })).toEqual({
      label: "Active",
      tone: "ok",
    });
  });

  it("reads scope off the directory that holds the file", () => {
    expect(isAllProjects(agent({}))).toBe(true);
    expect(isAllProjects(agent({ source: "project" }))).toBe(false);
  });

  const grouped = folderResolver(undefined, true);

  it("dots a draft amber regardless of phase", () => {
    const draft = { ...DEFAULT_OVERLAY, status: "draft" };
    expect(dotColor(agent({ description: "Spawn during the Plan stage" }), draft, grouped)).toBe(
      "var(--color-warn)",
    );
  });

  it("dots each phase with its own token, and Any stage / a custom folder with the muted colour", () => {
    const phrases: Record<string, string> = {
      Discover: "Spawn during the Research stage",
      Design: "Spawn during the Plan stage",
      Build: "Spawn during the Implement stage",
      Verify: "Spawn during the Validate stage",
      Deliver: "Spawn during the Commit stage",
    };
    for (const [phase, description] of Object.entries(phrases)) {
      expect(dotColor(agent({ description }), DEFAULT_OVERLAY, grouped)).toBe(
        `var(${PHASE_COLOR[phase as keyof typeof PHASE_COLOR]})`,
      );
    }
    expect(dotColor(agent({ description: "No stage named here." }), DEFAULT_OVERLAY, grouped)).toBe(
      MUTED,
    );
    expect(dotColor(agent({ category: "Spike" }), DEFAULT_OVERLAY, grouped)).toBe(MUTED);
  });

  it("dots an AGENT_PHASE agent with its phase token rather than the muted colour", () => {
    for (const [name, phase] of Object.entries(AGENT_PHASE)) {
      const dot = dotColor(agent({ name, description: "Names no stage." }), DEFAULT_OVERLAY, grouped);
      expect(dot).toBe(`var(${PHASE_COLOR[phase as keyof typeof PHASE_COLOR]})`);
    }
  });

  it("never resolves a folder name through Object.prototype", () => {
    for (const name of ["toString", "constructor", "hasOwnProperty", "__proto__"]) {
      expect(folderColor(name)).toBe(MUTED);
    }
  });
});

describe("PHASE_COLOR", () => {
  it("names a token for every phase SDLC_PHASES can colour (all but the Any stage catch-all)", () => {
    for (const phase of SDLC_PHASES) {
      if (phase === ANY_STAGE) continue;
      expect(PHASE_COLOR[phase as keyof typeof PHASE_COLOR]).toMatch(/^--color-/);
    }
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
    copy.tools.push("Bash");
    expect(source.tools).toEqual(["Read", "Grep"]);
  });

  it("lands a plugin agent's copy in your own library", () => {
    const copy = duplicateAgent(agent({ source: "plugin", plugin: "pr-review-toolkit" }));
    expect(copy.source).toBe("user");
    expect(copy.plugin).toBeNull();
  });

  it("turns a managed agent duplicate into a regular personal agent", () => {
    const copy = duplicateAgent(agent({ name: "cartographer", managed: true }));
    expect(copy.managed).toBe(false);
    expect(copy.source).toBe("user");
    expect(copy.path).toBe("");
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

describe("bundleChipState", () => {
  const SHA = "4c5cf370aed5fffee3b98f741bd6d911965c9033";
  const status = (over: Partial<AgentBundleStatus> = {}): AgentBundleStatus => ({
    manageable: true,
    installedSha: SHA,
    latestSha: null,
    installedVersion: "1.6.0",
    desiredVersion: "1.6.0",
    lastChecked: 1_700_000_000,
    updateAvailable: false,
    ...over,
  });
  const legacy = (over: Partial<AgentBundleStatus> = {}): AgentBundleStatus =>
    status({ installedVersion: null, desiredVersion: null, latestSha: SHA, ...over });
  const settled = { loading: false, error: false };

  it("up to date: names the installed version", () => {
    expect(bundleChipState(status(), settled)).toEqual({
      kind: "upToDate",
      label: "1.6.0",
      lastCheckedMs: 1_700_000_000_000,
      frozen: false,
    });
  });

  it("update available: names the version pair, not the SHA pair", () => {
    const s = status({ installedVersion: "1.5.9", updateAvailable: true });
    expect(bundleChipState(s, settled)).toEqual({
      kind: "updateAvailable",
      from: "1.5.9",
      to: "1.6.0",
    });
  });

  it("a versioned CLI over a bundle installed before versioned assets falls back to the SHA", () => {
    const s = status({ installedVersion: null, updateAvailable: true });
    expect(bundleChipState(s, settled)).toEqual({
      kind: "updateAvailable",
      from: "4c5cf37",
      to: "1.6.0",
    });
  });

  it("a pin is the target, so a bundle behind its pin reads as an update", () => {
    const s = status({ installedVersion: "1.6.0", desiredVersion: "1.5.9", updateAvailable: true });
    expect(bundleChipState(s, settled)).toEqual({
      kind: "updateAvailable",
      from: "1.6.0",
      to: "1.5.9",
    });
  });

  it("legacy CLI: shortens both SHAs", () => {
    const s = legacy({ latestSha: "9a1c2f0aed5fffee3b98f741bd6d911965c9033", updateAvailable: true });
    expect(bundleChipState(s, settled)).toEqual({
      kind: "updateAvailable",
      from: "4c5cf37",
      to: "9a1c2f0",
    });
  });

  it("legacy CLI, unknown latest (a failed probe) reads as up to date, not as an error", () => {
    const s = legacy({ latestSha: null, updateAvailable: false });
    expect(bundleChipState(s, settled)).toEqual({
      kind: "upToDate",
      label: "4c5cf37",
      lastCheckedMs: 1_700_000_000_000,
      frozen: true,
    });
  });

  it("marks a legacy CLI's up-to-date bundle as frozen", () => {
    expect(bundleChipState(legacy(), settled)).toMatchObject({ kind: "upToDate", frozen: true });
    expect(bundleChipState(status(), settled)).toMatchObject({ frozen: false });
  });

  it("a failed query is unknown, NOT a green up-to-date claim", () => {
    expect(bundleChipState(undefined, { loading: false, error: true }).kind).toBe("unknown");
    expect(bundleChipState(undefined, settled).kind).toBe("unknown");
  });

  it("still checking is unknown, so the first render makes no claim either", () => {
    expect(bundleChipState(undefined, { loading: true, error: false })).toEqual({
      kind: "unknown",
      detail: "Checking for agent updates…",
    });
  });

  it("a tool this app cannot manage is unknown, not an update prompt", () => {
    const s = status({ manageable: false, installedSha: null, updateAvailable: false });
    expect(bundleChipState(s, settled).kind).toBe("unknown");
  });

  it("never installed but manageable: no identity, no last-checked time, up to date", () => {
    const s = legacy({ installedSha: null, latestSha: null, lastChecked: null });
    expect(bundleChipState(s, settled)).toEqual({
      kind: "upToDate",
      label: null,
      lastCheckedMs: null,
      frozen: true,
    });
  });

  it("converts lastChecked from Unix seconds to the epoch-ms formatRelativeTime expects", () => {
    const at = (over: Partial<AgentBundleStatus>) => {
      const state = bundleChipState(status(over), settled);
      return state.kind === "upToDate" ? state.lastCheckedMs : undefined;
    };
    expect(at({ lastChecked: 0 })).toBe(0);
    expect(at({ lastChecked: null })).toBeNull();
  });
});
