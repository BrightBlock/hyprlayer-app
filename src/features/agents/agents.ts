/**
 * Agent-library pure core.
 *
 * An agent IS a `.claude/agents/<name>.md` file, frontmatter for the launch
 * config, body for the system prompt. The launch settings claude's frontmatter
 * has no key for (effort, harness, permissions, rules, folder) ride alongside
 * in `AppSettings.agents`, keyed by the file's path.
 *
 * Shared vocabulary (models, efforts, harnesses, permission modes) comes from
 * `lib/domain`, never re-declared here.
 */

import type {
  AgentBundleReport,
  AgentBundleStatus,
  AgentFile,
  AgentLibrarySettings,
  AgentOverlay,
  AgentRule,
  AgentSkill,
} from "@/lib/bindings";
import { MODELS } from "@/lib/domain/models";
import { treeIndent } from "@/lib/layout";
import {
  AGENT_PHASE,
  ANY_STAGE,
  PHASE_COLOR,
  SDLC_PHASES,
  type SdlcPhase,
  STAGE_PHASE,
  STAGES,
  type StageKey,
} from "@/lib/domain/stages";

/** The frontmatter `model` value for "whatever spawned me", claude's own
 *  spelling, and what an absent key means. */
export const INHERIT_MODEL = "inherit";

/** Display name for a frontmatter `model`. An unknown value is shown as-is,
 *  claude accepts full model ids there too. */
export function modelLabel(model: string | null): string {
  if (!model || model === INHERIT_MODEL) return "Inherit";
  const option = MODELS.find((m) => m.key === model);
  return option ? `${option.label}${option.hint ? ` (${option.hint})` : ""}` : model;
}

/** How much of a description the list row shows before eliding. Long enough for
 *  a real "use this when …" sentence, short enough that the row stays scannable. */
const DESCRIPTION_MAX = 90;

/**
 * The one-line form of an agent's description. Frontmatter descriptions run to
 * whole paragraphs, trigger lists, examples, so the row shows the first
 * sentence, capped, rather than a wall of text ellipsised by the column width.
 * The full text stays available as the row's tooltip.
 */
export function shortDescription(description: string): string {
  const flat = description.replace(/\s+/g, " ").trim();
  const text = flat.match(/^.*?[.!?](?=\s|$)/)?.[0] ?? flat;
  if (text.length <= DESCRIPTION_MAX) return text;
  const cut = text.slice(0, DESCRIPTION_MAX);
  const space = cut.lastIndexOf(" ");
  const kept = space > DESCRIPTION_MAX / 2 ? cut.slice(0, space) : cut;
  return `${kept.replace(/[,;:.\s]+$/, "")}…`;
}

/**
 * Claude's `memory` frontmatter scopes, in the order the editor offers them.
 * An absent key is "off", the agent starts every invocation with nothing
 * remembered, so "off" is not a scope value. It is the key's absence.
 */
export const MEMORY_SCOPES: ReadonlyArray<{ key: string; label: string; hint: string }> = [
  { key: "local", label: "Session", hint: "Remembers within one session, forgotten after" },
  { key: "project", label: "Project", hint: "Kept in this repo's .claude/agent-memory/<name>/" },
  { key: "user", label: "Everywhere", hint: "Follows the agent across every project" },
];

/** Display name for a `memory` scope. An unrecognized scope, a newer claude,
 *  or a hand-edited file, is shown as-is rather than silently called "off". */
export function memoryLabel(memory: string | null): string {
  if (!memory) return "Off";
  return MEMORY_SCOPES.find((s) => s.key === memory)?.label ?? memory;
}

/** True when the file carries a scope this editor doesn't offer, so the UI can
 *  say so instead of pretending one of its own radios is selected. */
export function hasUnknownMemoryScope(memory: string | null): boolean {
  return !!memory && !MEMORY_SCOPES.some((s) => s.key === memory);
}

/** Which list the top switch shows. System = agents an installed plugin ships. */
export type AgentTab = "yours" | "system";
/** Global lists every agent you own; project narrows to the repo's own. */
export type AgentScope = "global" | "project";
/** The sidebar's root nav choice — `?section=` in the URL. `"system"` maps to
 *  `AgentTab`'s `"system"`; `"skills"` has no `AgentTab` equivalent, the
 *  placeholder root has no list to filter. */
export type AgentSection = "agents" | "system" | "skills";

export type EditorTab = "setup" | "prompt" | "capabilities" | "permissions" | "collaborators";

export const EDITOR_TABS: ReadonlyArray<{ key: EditorTab; label: string }> = [
  { key: "setup", label: "Setup" },
  { key: "prompt", label: "Prompt" },
  { key: "capabilities", label: "Capabilities" },
  { key: "permissions", label: "Permissions" },
  { key: "collaborators", label: "Collaborators" },
];

/** Sharing an agent needs a Hyprlayer account, so that tab is sign-in gated. */
export function editorTabs(signedIn: boolean): ReadonlyArray<{ key: EditorTab; label: string }> {
  return signedIn ? EDITOR_TABS : EDITOR_TABS.filter((t) => t.key !== "collaborators");
}

/**
 * The built-in tools an agent's frontmatter can name. Claude Code accepts any
 * tool id, MCP globs included, so this is the set the editor offers as
 * toggles, not a closed vocabulary: anything else in the file is preserved.
 */
export const AGENT_TOOLS: ReadonlyArray<{ key: string; desc: string }> = [
  { key: "Read", desc: "Read files in the worktree" },
  { key: "Write", desc: "Create new files" },
  { key: "Edit", desc: "Modify existing files" },
  { key: "Bash", desc: "Run shell commands" },
  { key: "Glob", desc: "Find files by pattern" },
  { key: "Grep", desc: "Search file contents" },
  { key: "WebFetch", desc: "Fetch a URL" },
  { key: "WebSearch", desc: "Search the web" },
  { key: "Task", desc: "Delegate to a sub-agent" },
  { key: "TodoWrite", desc: "Keep a visible task list" },
  { key: "NotebookEdit", desc: "Edit Jupyter notebooks" },
  { key: "SlashCommand", desc: "Run a saved prompt" },
  { key: "Skill", desc: "Invoke a skill" },
];

/** The tools a read-only agent keeps. */
const READ_ONLY_TOOLS = ["Read", "Glob", "Grep"];

/** An empty `tools` list means the agent inherits every tool its parent has,
 *  materially different from an agent with none, so the UI says so. */
export function inheritsAllTools(agent: AgentFile): boolean {
  return agent.tools.length === 0;
}

export function toolEnabled(agent: AgentFile, key: string): boolean {
  return inheritsAllTools(agent) || agent.tools.includes(key);
}

/** Turning one tool off has to spell out the rest, since the file expresses
 *  "everything" by omission. Entries the editor doesn't model ride along. */
export function setTool(agent: AgentFile, key: string, on: boolean): string[] {
  const base = inheritsAllTools(agent) ? AGENT_TOOLS.map((t) => t.key) : agent.tools;
  if (!on) return base.filter((t) => t !== key);
  return base.includes(key) ? [...base] : [...base, key];
}

export function readOnlyTools(agent: AgentFile): string[] {
  const extras = agent.tools.filter((t) => !AGENT_TOOLS.some((x) => x.key === t));
  return [...READ_ONLY_TOOLS, ...extras];
}

/** "5 of 12 enabled …" for the Capabilities header. */
export function toolSummary(agent: AgentFile): string {
  if (inheritsAllTools(agent)) {
    return "Inherits every tool from the session that spawns it. Turn one off to pin an explicit list";
  }
  const known = agent.tools.filter((t) => AGENT_TOOLS.some((x) => x.key === t)).length;
  return `${known} of ${AGENT_TOOLS.length} enabled · disabled tools are hidden from the model entirely`;
}

/** Tool entries the file names that aren't built-ins or MCP globs, anything
 *  from a newer claude. Surfaced so a save can't look like it dropped them. */
export function extraTools(tools: readonly string[]): string[] {
  return tools.filter(
    (t) => !t.startsWith("mcp__") && !AGENT_TOOLS.some((x) => x.key === t),
  );
}

/** An agent may use `server` when its tools name any of that server's tools. */
export function mcpEnabled(agent: AgentFile, server: string): boolean {
  return agent.tools.some((t) => t.startsWith(`mcp__${server}__`));
}

/** The server's tool entries this agent names, or the wildcard when enabled
 *  wholesale. Empty when the server is off. */
export function mcpTools(agent: AgentFile, server: string): string[] {
  return agent.tools.filter((t) => t.startsWith(`mcp__${server}__`));
}

export function setMcp(agent: AgentFile, server: string, on: boolean): string[] {
  const base = inheritsAllTools(agent) ? AGENT_TOOLS.map((t) => t.key) : agent.tools;
  const without = base.filter((t) => !t.startsWith(`mcp__${server}__`));
  return on ? [...without, `mcp__${server}__*`] : without;
}

export function skillEnabled(agent: AgentFile, name: string): boolean {
  return agent.skills.includes(name);
}

/** Turning a skill on also has to leave the agent able to invoke skills. An
 *  agent inheriting every tool already can, so its `tools` stays empty. */
export function setSkill(
  agent: AgentFile,
  name: string,
  on: boolean,
): { skills: string[]; tools: string[] } {
  const skills = on
    ? agent.skills.includes(name)
      ? [...agent.skills]
      : [...agent.skills, name]
    : agent.skills.filter((s) => s !== name);
  const tools = on && !inheritsAllTools(agent) ? setTool(agent, "Skill", true) : [...agent.tools];
  return { skills, tools };
}

/** Names the file lists that discovery didn't match, a newer claude, a plugin
 *  that isn't installed here, or a typo. Shown rather than silently dropped. */
export function extraSkills(skills: readonly string[], known: readonly AgentSkill[]): string[] {
  return skills.filter((s) => !known.some((k) => k.name === s));
}

/** "2 of 9 preloaded · ~7.0k tokens added per spawn" for the card header.
 *  Uses the same ~3.8 chars/token ratio as `promptStats` (agents.ts:311). */
export function skillSummary(agent: AgentFile, known: readonly AgentSkill[]): string {
  const enabled = known.filter((k) => agent.skills.includes(k.name));
  const chars = enabled.reduce((sum, k) => sum + k.chars, 0);
  const tokensK = chars / 3.8 / 1000;
  return `${enabled.length} of ${known.length} preloaded · ~${tokensK.toFixed(1)}k tokens added per spawn`;
}

/** True when a `deny Skill` catch-all is present, the rules are the source of
 *  truth for "only these skills", so there is no separate overlay field. */
export function skillsRestricted(rules: readonly AgentRule[]): boolean {
  return rules.some((r) => r.action === "deny" && r.pattern === "Skill");
}

/** Matches the managed block: bare `Skill` or a single `Skill(<name>)` entry. */
const SKILL_RULE_PATTERN = /^Skill(\([^)]*\))?$/;

/** Rewrites only the rules whose pattern is `Skill` or `Skill(<name>)`: one
 *  `allow Skill(<name>)` per enabled skill, then `deny Skill` when restricted.
 *  User rules keep their content and relative order. */
export function syncSkillRules(
  rules: readonly AgentRule[],
  skills: readonly string[],
  restrict: boolean,
): AgentRule[] {
  const survivors = rules.filter((r) => !SKILL_RULE_PATTERN.test(r.pattern));
  let nextId = survivors.reduce((max, r) => Math.max(max, Number(r.id.slice(1)) || 0), 0) + 1;
  const generated: AgentRule[] = skills.map((name) => ({
    id: `r${nextId++}`,
    action: "allow",
    pattern: `Skill(${name})`,
  }));
  if (restrict) generated.push({ id: `r${nextId++}`, action: "deny", pattern: "Skill" });
  return [...survivors, ...generated];
}

/** A blank agent for `source`. `path` is assigned by the backend once the name
 *  is known, so an unsaved agent carries an empty one. */
export function newAgent(source: string): AgentFile {
  return {
    path: "",
    source,
    plugin: null,
    name: "",
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
  };
}

/** True while an agent has never been written, the editor's "new" state. */
export function isUnsaved(agent: AgentFile): boolean {
  return agent.path === "";
}

/** A plugin ships this agent; its file is replaced on the plugin's next update,
 *  so edits are saved as a personal override instead. */
export function isSystem(agent: AgentFile): boolean {
  return agent.source === "plugin";
}

/** A personal agent is available everywhere; a project one is scoped to its
 *  repo. Which directory holds the file IS the scope. */
export function isAllProjects(agent: AgentFile): boolean {
  return agent.source !== "project";
}

/** An overlay with every field present, what the editor works against. The
 *  generated type has them all optional (serde `default`). */
export type Overlay = Required<AgentOverlay>;

export const DEFAULT_OVERLAY: Overlay = {
  effort: "high",
  harness: "claude",
  folder: "",
  status: "published",
  permissionMode: "default",
  rules: [],
  network: true,
  outsideWorktree: false,
  context: [],
  delegates: [],
  visibility: "private",
  collaborators: [],
};

export function overlayFor(library: AgentLibrarySettings | undefined, path: string): Overlay {
  return { ...DEFAULT_OVERLAY, ...(library?.overlays?.[path] ?? {}) };
}

/** Status badge text and tone. A plugin agent is always live; yours is a draft
 *  until published, and a draft can't be spawned by name in a session, not
 *  just hidden from the composer (see `agent_deny_rules`, applied at launch). */
export function statusOf(
  agent: AgentFile,
  overlay: Overlay,
): { label: string; tone: "ok" | "warn" } {
  if (isSystem(agent)) return { label: "Active", tone: "ok" };
  return overlay.status === "draft"
    ? { label: "Draft", tone: "warn" }
    : { label: "Published", tone: "ok" };
}

const MUTED_DOT = "color-mix(in oklab, var(--color-ink-dim) 55%, transparent)";

/** A tight phrase, not a bare keyword, an agent whose description merely
 *  mentions a stage word in passing (e.g. "Drafts an implementation plan
 *  body") must not be filed under that stage's phase. Built from `STAGES` so
 *  a stage added to the Rust enum needs no edit here. */
const STAGE_PHRASE = new RegExp(
  String.raw`\b(?:during|in)\s+the\s+(${STAGES.map((s) => s.key).join("|")})\s+stage\b`,
  "i",
);

/** A `Map`, not [`AGENT_PHASE`] itself: `name` is free text out of a user's
 *  frontmatter, so `AGENT_PHASE["toString"]` would resolve to
 *  `Object.prototype.toString` and file the agent under a function. Same
 *  reason [`PHASE_TOKEN`] below is a `Map`. */
const NAME_PHASE = new Map<string, SdlcPhase>(Object.entries(AGENT_PHASE));

/** The phase an agent's description names via the shipped agents' convention
 *  ("Spawn during the <Stage> stage"), else the one [`AGENT_PHASE`] assigns
 *  its name, else [`ANY_STAGE`]. The description wins: an agent that states
 *  its own stage is authoritative over a table it never sees. */
export function phaseFolder(agent: AgentFile): string {
  const match = agent.description.match(STAGE_PHRASE);
  if (match) return STAGE_PHASE[match[1].toLowerCase() as StageKey] ?? ANY_STAGE;
  return NAME_PHASE.get(agent.name) ?? ANY_STAGE;
}

/**
 * An agent's folder, highest precedence first:
 * - plugin agent: `overlay.folder` (non-empty) → its plugin's name
 * - owned agent: `category` (non-empty) → `overlay.folder` (non-empty,
 *   legacy) → [`phaseFolder`], when `groupUncategorized` is on → loose (`""`)
 *
 * `category` stays at the top even though we no longer ship it: a
 * user-authored agent may still set it, and it is theirs to set.
 *
 * Prefer [`folderResolver`] at a call site that has a whole list: it binds the
 * library and the flag once instead of threading both through every layer.
 */
export function folderOf(agent: AgentFile, overlay: Overlay, groupUncategorized: boolean): string {
  if (isSystem(agent)) return overlay.folder || agent.plugin || "";
  if (agent.category) return agent.category;
  if (overlay.folder) return overlay.folder;
  return groupUncategorized ? phaseFolder(agent) : "";
}

/** The folder policy as one function of an agent. Everything downstream of the
 *  library takes this rather than the `(library, groupUncategorized)` pair, so
 *  the grouping flag stops travelling and `overlayFor` resolves in one place. */
export type FolderOf = (agent: AgentFile) => string;

export function folderResolver(
  library: AgentLibrarySettings | undefined,
  groupUncategorized: boolean,
): FolderOf {
  return (agent) => folderOf(agent, overlayFor(library, agent.path), groupUncategorized);
}

/** Where a folder sorts: phases in [`SDLC_PHASES`] order (which puts
 *  [`ANY_STAGE`] last among them), then everything else after. */
const PHASE_RANK = new Map<string, number>(SDLC_PHASES.map((phase, i) => [phase, i]));

/**
 * Every folder name in play, for the list and both folder pickers: the phases
 * first in phase order, then everything else, occupied `category` values,
 * plugin names, and `library.folders`, alphabetically.
 *
 * A persisted folder named like a phase sorts with the phases rather than
 * being dropped: it is the same bucket, and omitting it would let the create
 * dialog accept a name that then never appeared anywhere.
 */
export function folderList(
  agents: readonly AgentFile[],
  library: AgentLibrarySettings | undefined,
  groupUncategorized: boolean,
): string[] {
  const folderOfAgent = folderResolver(library, groupUncategorized);
  const occupied = agents.map(folderOfAgent).filter((folder) => folder !== "");
  const named = [...new Set([...occupied, ...(library?.folders ?? [])])];
  const [phases, rest] = partition(named, (folder) => PHASE_RANK.has(folder));
  return [
    ...phases.sort((a, b) => (PHASE_RANK.get(a) ?? 0) - (PHASE_RANK.get(b) ?? 0)),
    ...rest.sort((a, b) => a.localeCompare(b)),
  ];
}

function partition<T>(items: readonly T[], pick: (item: T) => boolean): [T[], T[]] {
  const yes: T[] = [];
  const no: T[] = [];
  for (const item of items) (pick(item) ? yes : no).push(item);
  return [yes, no];
}

/** A `Map`, not the record itself: a folder name is free text out of a user's
 *  frontmatter, and indexing an object literal with it would let
 *  `PHASE_COLOR["toString"]` resolve to `Object.prototype.toString` and emit
 *  `var(function toString() { … })` as a CSS value. */
const PHASE_TOKEN = new Map<string, string>(Object.entries(PHASE_COLOR));

/** The colour a folder itself paints with, the header icon and, via
 *  [`dotColor`], every agent filed under it. `MUTED_DOT` for "Any stage" and
 *  any custom folder that isn't a phase name. */
export function folderColor(folder: string): string {
  const token = PHASE_TOKEN.get(folder);
  return token ? `var(${token})` : MUTED_DOT;
}

/** The dot beside an agent's name: amber while it is a draft, otherwise the
 *  colour of the folder it sits in. */
export function dotColor(agent: AgentFile, overlay: Overlay, folderOfAgent: FolderOf): string {
  if (!isSystem(agent) && overlay.status === "draft") return "var(--color-warn)";
  return folderColor(folderOfAgent(agent));
}

/** The agents a tab + scope + search admits, in the order the list renders. */
export function visibleAgents(
  agents: readonly AgentFile[],
  filter: { tab: AgentTab; scope: AgentScope; search: string },
): AgentFile[] {
  const q = filter.search.trim().toLowerCase();
  return agents.filter((a) => {
    if (isSystem(a) !== (filter.tab === "system")) return false;
    if (filter.tab === "yours" && filter.scope === "project" && a.source !== "project") return false;
    if (!q) return true;
    return a.name.toLowerCase().includes(q) || a.description.toLowerCase().includes(q);
  });
}

/** A folder header, or an agent. The list renders these in order. */
export type AgentRow =
  | { kind: "group"; folder: string; count: number; open: boolean }
  | { kind: "agent"; agent: AgentFile; indent: number };

/**
 * Loose agents first, then each folder that has something in it. A collapsed
 * folder still renders its header; an empty one is dropped rather than shown
 * as a dead row, unless `keepEmpty` names it.
 */
export function agentRows({
  agents,
  folders,
  closedFolders,
  folderOf: folderOfAgent,
  keepEmpty,
}: {
  agents: readonly AgentFile[];
  /** Every folder name, in render order, see [`folderList`]. */
  folders: readonly string[];
  /** Folders the user has collapsed. Membership means closed; absence, a
   *  folder never touched, including one that appears later, means open. */
  closedFolders: ReadonlySet<string>;
  folderOf: FolderOf;
  /** Folders that render even with nothing in them — `library.folders`, the
   *  ones the user created by hand. Without this "New folder…" wrote a name
   *  into settings that the tree then refused to draw, because a folder is
   *  empty for exactly as long as it takes to move the first agent into it,
   *  and the only picker that could do the moving is the one place the name
   *  did show up. The derived folders (phases, plugin names, `category`
   *  values) are deliberately not in here: they exist only because an agent
   *  is in them, so "empty" and "gone" are the same state. */
  keepEmpty?: ReadonlySet<string>;
}): AgentRow[] {
  const loose: AgentRow[] = [];
  const inside = new Map<string, AgentFile[]>();
  for (const agent of agents) {
    const folder = folderOfAgent(agent);
    if (!folder) {
      loose.push({ kind: "agent", agent, indent: treeIndent(0, 0) });
      continue;
    }
    const members = inside.get(folder);
    if (members) members.push(agent);
    else inside.set(folder, [agent]);
  }
  const grouped = folders.flatMap((folder): AgentRow[] => {
    const members = inside.get(folder) ?? [];
    if (members.length === 0 && !keepEmpty?.has(folder)) return [];
    const open = !closedFolders.has(folder);
    const header: AgentRow = { kind: "group", folder, count: members.length, open };
    if (!open) return [header];
    return [
      header,
      ...members.map((agent): AgentRow => ({ kind: "agent", agent, indent: treeIndent(1, 0) })),
    ];
  });
  return [...loose, ...grouped];
}

/** Length + a rough token count for the prompt editor's counter. ~3.8 chars per
 *  token is the usual English ratio; exact counting needs the tokenizer. */
export function promptStats(prompt: string): string {
  return `${prompt.length} chars · ~${Math.max(1, Math.round(prompt.length / 3.8))} tokens`;
}

/** A copy of `agent` under a new name, unsaved until it is given a path. A
 *  plugin agent duplicates into your own library, losing its plugin origin. */
export function duplicateAgent(agent: AgentFile): AgentFile {
  return {
    ...structuredClone(agent),
    path: "",
    plugin: null,
    source: isSystem(agent) ? "user" : agent.source,
    name: `${agent.name} copy`,
  };
}

/** The blank rule "Add rule" appends. Ids only have to be unique within the
 *  agent, and the list is short. */
export function newRule(existing: readonly { id: string }[]): {
  id: string;
  action: string;
  pattern: string;
} {
  const next = existing.reduce((max, r) => Math.max(max, Number(r.id.slice(1)) || 0), 0) + 1;
  return { id: `r${next}`, action: "ask", pattern: "Bash(:*)" };
}

/** Name and initials for an invited email, the way the People list shows them. */
export function collaboratorFor(email: string): {
  name: string;
  email: string;
  role: string;
} {
  const name = email.split("@")[0].replace(/[._-]+/g, " ").trim();
  return { name: name || email, email, role: "Editor" };
}

export function initialsOf(name: string): string {
  return name
    .split(/\s+/)
    .filter(Boolean)
    .map((w) => w[0])
    .join("")
    .slice(0, 2)
    .toUpperCase();
}

/** Result of one `updateAgentBundle` run, the data behind `AgentUpdateReport`.
 *  The SHA pair rides alongside the backend's report because the command
 *  cannot report it: `ai reinstall` prints no SHA, so the pair is the one the
 *  chip was showing when the user triggered the update. Either half may be
 *  `null` — a retry fired from the failure dialog can run after the freshness
 *  query itself has stopped answering — and the dialog then says nothing
 *  about SHAs rather than printing a sentinel where one belongs. */
export type BundleReport = AgentBundleReport & {
  fromSha: string | null;
  toSha: string | null;
};

/** A failed `updateAgentBundle` run, the data behind `AgentUpdateFailed`. */
export type BundleFailure = {
  message: string;
  /** The SHA the library is still on, unchanged, so the dialog can say so. */
  stillOnSha: string | null;
};

/** The header chip's states. "Updating" is a mutation in flight, not a shape
 *  of `AgentBundleStatus`, so it lives at the call site rather than here. */
export type BundleChipState =
  /** No verdict to give: still checking, the query failed, or the CLI reports
   *  a tool whose bundle this app does not manage. */
  | { kind: "unknown"; detail: string }
  | { kind: "upToDate"; sha: string | null; lastCheckedMs: number | null }
  | { kind: "updateAvailable"; fromSha: string | null; toSha: string };

/**
 * Maps the bundle-freshness probe to the chip's state, so the component only
 * ever switches on `kind`.
 *
 * The `unknown` state is load-bearing, not a placeholder: `agentBundleStatus`
 * errors outright when the `hyprlayer` binary is missing or its JSON won't
 * parse, and folding that `undefined` into `upToDate` (as this did) paints a
 * green dot asserting a bundle is current that the app could not read one
 * byte of. `manageable: false` lands here for the same reason — the CLI
 * answers `{}` for an unconfigured machine, which is not a stale bundle.
 *
 * A failed *probe* (`manageable`, but `latestSha: null`) still reads as up to
 * date: not knowing whether a newer bundle exists is not the same as being
 * unable to look. `lastChecked` arrives in Unix seconds, the CLI's
 * convention; converted to epoch ms here so `formatRelativeTime` needs no
 * unit special-casing at the call site.
 */
export function bundleChipState(
  status: AgentBundleStatus | undefined,
  query: { loading: boolean; error: boolean },
): BundleChipState {
  if (query.loading) return { kind: "unknown", detail: "Checking for agent updates…" };
  if (query.error || !status) {
    return { kind: "unknown", detail: "Could not check — the hyprlayer CLI did not answer." };
  }
  if (!status.manageable) {
    return { kind: "unknown", detail: "No Claude Code agent bundle is configured." };
  }
  const lastCheckedMs = status.lastChecked != null ? status.lastChecked * 1000 : null;
  // Narrowed on `latestSha` rather than on `updateAvailable`, so `toSha` is a
  // `string` by control flow and needs no unreachable fallback sentinel.
  const latest = status.latestSha;
  if (!status.updateAvailable || latest === null) {
    return { kind: "upToDate", sha: status.installedSha, lastCheckedMs };
  }
  return { kind: "updateAvailable", fromSha: status.installedSha, toSha: latest };
}
