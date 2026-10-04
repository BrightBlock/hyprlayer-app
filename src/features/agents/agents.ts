
import type {
  AgentBundleReport,
  AgentBundleStatus,
  AgentFile,
  AgentLibrarySettings,
  AgentOverlay,
  AgentRule,
  AgentSkill,
} from "@/lib/bindings";
import { MODELS, type ModelOption } from "@/lib/domain/models";
import { plural, shortSha } from "@/lib/format";
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

export const INHERIT_MODEL = "inherit";

export function modelLabel(
  model: string | null,
  catalog: readonly ModelOption[] = MODELS,
): string {
  if (!model || model === INHERIT_MODEL) return "Inherit";
  const option = catalog.find((m) => m.key === model) ?? MODELS.find((m) => m.key === model);
  return option ? `${option.label}${option.hint ? ` (${option.hint})` : ""}` : model;
}

const DESCRIPTION_MAX = 90;

export function shortDescription(description: string): string {
  const flat = description.replace(/\s+/g, " ").trim();
  const text = flat.match(/^.*?[.!?](?=\s|$)/)?.[0] ?? flat;
  if (text.length <= DESCRIPTION_MAX) return text;
  const cut = text.slice(0, DESCRIPTION_MAX);
  const space = cut.lastIndexOf(" ");
  const kept = space > DESCRIPTION_MAX / 2 ? cut.slice(0, space) : cut;
  return `${kept.replace(/[,;:.\s]+$/, "")}…`;
}

export const MEMORY_SCOPES: ReadonlyArray<{ key: string; label: string; hint: string }> = [
  { key: "local", label: "Session", hint: "Remembers within one session, forgotten after" },
  { key: "project", label: "Project", hint: "Kept in this repo's .claude/agent-memory/<name>/" },
  { key: "user", label: "Everywhere", hint: "Follows the agent across every project" },
];

export function hasUnknownMemoryScope(memory: string | null): boolean {
  return !!memory && !MEMORY_SCOPES.some((s) => s.key === memory);
}

export type AgentTab = "yours" | "system";
export type AgentScope = "global" | "project";
export type AgentSection = "agents" | "system";

export type EditorTab = "setup" | "prompt" | "capabilities" | "permissions" | "collaborators";

export const EDITOR_TABS: ReadonlyArray<{ key: EditorTab; label: string }> = [
  { key: "setup", label: "Setup" },
  { key: "prompt", label: "Prompt" },
  { key: "capabilities", label: "Capabilities" },
  { key: "permissions", label: "Permissions" },
  { key: "collaborators", label: "Collaborators" },
];

export function editorTabs(signedIn: boolean): ReadonlyArray<{ key: EditorTab; label: string }> {
  return signedIn ? EDITOR_TABS : EDITOR_TABS.filter((t) => t.key !== "collaborators");
}

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

const READ_ONLY_TOOLS = ["Read", "Glob", "Grep"];

export function inheritsAllTools(agent: AgentFile): boolean {
  return agent.tools.length === 0;
}

export function toolEnabled(agent: AgentFile, key: string): boolean {
  return inheritsAllTools(agent) || agent.tools.includes(key);
}

export function setTool(agent: AgentFile, key: string, on: boolean): string[] {
  const base = inheritsAllTools(agent) ? AGENT_TOOLS.map((t) => t.key) : agent.tools;
  if (!on) return base.filter((t) => t !== key);
  return base.includes(key) ? [...base] : [...base, key];
}

export function readOnlyTools(agent: AgentFile): string[] {
  const extras = agent.tools.filter((t) => !AGENT_TOOLS.some((x) => x.key === t));
  return [...READ_ONLY_TOOLS, ...extras];
}

export function toolSummary(agent: AgentFile): string {
  if (inheritsAllTools(agent)) {
    return "Inherits every tool from the session that spawns it. Turn one off to pin an explicit list";
  }
  const known = agent.tools.filter((t) => AGENT_TOOLS.some((x) => x.key === t)).length;
  return `${known} of ${AGENT_TOOLS.length} enabled · disabled tools are hidden from the model entirely`;
}

export function extraTools(tools: readonly string[]): string[] {
  return tools.filter(
    (t) => !t.startsWith("mcp__") && !AGENT_TOOLS.some((x) => x.key === t),
  );
}

export function mcpEnabled(agent: AgentFile, server: string): boolean {
  return agent.tools.some((t) => t.startsWith(`mcp__${server}__`));
}

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

export function extraSkills(skills: readonly string[], known: readonly AgentSkill[]): string[] {
  return skills.filter((s) => !known.some((k) => k.name === s));
}

export function skillSummary(agent: AgentFile, known: readonly AgentSkill[]): string {
  const enabled = known.filter((k) => agent.skills.includes(k.name));
  const chars = enabled.reduce((sum, k) => sum + k.chars, 0);
  const tokensK = chars / 3.8 / 1000;
  return `${enabled.length} of ${known.length} preloaded · ~${tokensK.toFixed(1)}k tokens added per spawn`;
}

export function skillsRestricted(rules: readonly AgentRule[]): boolean {
  return rules.some((r) => r.action === "deny" && r.pattern === "Skill");
}

const SKILL_RULE_PATTERN = /^Skill(\([^)]*\))?$/;

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

export function newAgent(source: string): AgentFile {
  return {
    path: "",
    source,
    plugin: null,
    managed: false,
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

export function isUnsaved(agent: AgentFile): boolean {
  return agent.path === "";
}

export function isSystem(agent: AgentFile): boolean {
  return agent.source === "plugin" || agent.managed === true;
}

export function isManaged(agent: AgentFile): boolean {
  return agent.managed === true;
}

export function isAllProjects(agent: AgentFile): boolean {
  return agent.source !== "project";
}

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

const STAGE_PHRASE = new RegExp(
  String.raw`\b(?:during|in)\s+the\s+(${STAGES.map((s) => s.key).join("|")})\s+stage\b`,
  "i",
);

const NAME_PHASE = new Map<string, SdlcPhase>(Object.entries(AGENT_PHASE));

export function phaseFolder(agent: AgentFile): string {
  const match = agent.description.match(STAGE_PHRASE);
  if (match) return STAGE_PHASE[match[1].toLowerCase() as StageKey] ?? ANY_STAGE;
  return NAME_PHASE.get(agent.name) ?? ANY_STAGE;
}

export function folderOf(agent: AgentFile, overlay: Overlay, groupUncategorized: boolean): string {
  if (isSystem(agent)) return overlay.folder || agent.plugin || "";
  if (agent.category) return agent.category;
  if (overlay.folder) return overlay.folder;
  return groupUncategorized ? phaseFolder(agent) : "";
}

export type FolderOf = (agent: AgentFile) => string;

export function folderResolver(
  library: AgentLibrarySettings | undefined,
  groupUncategorized: boolean,
): FolderOf {
  return (agent) => folderOf(agent, overlayFor(library, agent.path), groupUncategorized);
}

// System agents file under SDLC phases like yours; the plugin name moves to the row meta.
export function systemFolderOf(agent: AgentFile, library: AgentLibrarySettings | undefined): string {
  return overlayFor(library, agent.path).folder || phaseFolder(agent);
}

export function systemFolderResolver(library: AgentLibrarySettings | undefined): FolderOf {
  return (agent) => systemFolderOf(agent, library);
}

export function stagesForPhase(folder: string): StageKey[] {
  return STAGES.filter((s) => STAGE_PHASE[s.key] === folder).map((s) => s.key);
}

export function agentMeta(agent: AgentFile, catalog: readonly ModelOption[]): string {
  const tools = inheritsAllTools(agent) ? "all tools" : plural(agent.tools.length, "tool");
  return `${modelLabel(agent.model, catalog)} · ${tools} · ${plural(agent.skills.length, "skill")}`;
}

const PHASE_RANK = new Map<string, number>(SDLC_PHASES.map((phase, i) => [phase, i]));

export function folderList(
  agents: readonly AgentFile[],
  library: AgentLibrarySettings | undefined,
  groupUncategorized: boolean,
  folderOfAgent: FolderOf = folderResolver(library, groupUncategorized),
): string[] {
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

const PHASE_TOKEN = new Map<string, string>(Object.entries(PHASE_COLOR));

export function folderColor(folder: string): string {
  const token = PHASE_TOKEN.get(folder);
  return token ? `var(${token})` : MUTED_DOT;
}

export function dotColor(agent: AgentFile, overlay: Overlay, folderOfAgent: FolderOf): string {
  if (!isSystem(agent) && overlay.status === "draft") return "var(--color-warn)";
  return folderColor(folderOfAgent(agent));
}

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

export type AgentRow =
  | { kind: "group"; folder: string; count: number; open: boolean }
  | { kind: "agent"; agent: AgentFile; indent: number };

export function agentRows({
  agents,
  folders,
  closedFolders,
  folderOf: folderOfAgent,
  keepEmpty,
}: {
  agents: readonly AgentFile[];
  folders: readonly string[];
  closedFolders: ReadonlySet<string>;
  folderOf: FolderOf;
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

export function promptStats(prompt: string): string {
  return `${prompt.length} chars · ~${Math.max(1, Math.round(prompt.length / 3.8))} tokens`;
}

export function duplicateAgent(agent: AgentFile): AgentFile {
  return {
    ...structuredClone(agent),
    path: "",
    plugin: null,
    managed: false,
    source: isSystem(agent) ? "user" : agent.source,
    name: `${agent.name} copy`,
  };
}

export function newRule(existing: readonly { id: string }[]): {
  id: string;
  action: string;
  pattern: string;
} {
  const next = existing.reduce((max, r) => Math.max(max, Number(r.id.slice(1)) || 0), 0) + 1;
  return { id: `r${next}`, action: "ask", pattern: "Bash(:*)" };
}

export function collaboratorFor(email: string): {
  name: string;
  email: string;
  role: string;
} {
  const name = email.split("@")[0].replace(/[._-]+/g, " ").trim();
  return { name: name || email, email, role: "Editor" };
}

export { initialsOf } from "@/lib/format";

export type BundleReport = AgentBundleReport & {
  from: string | null;
  to: string | null;
};

export type BundleFailure = {
  message: string;
  stillOn: string | null;
};

export type BundleChipState =
  | { kind: "unknown"; detail: string }
  | {
      kind: "upToDate";
      label: string | null;
      lastCheckedMs: number | null;
      frozen: boolean;
    }
  | { kind: "updateAvailable"; from: string | null; to: string };

export function bundleChipState(
  status: AgentBundleStatus | undefined,
  query: { loading: boolean; error: boolean },
): BundleChipState {
  if (query.loading) return { kind: "unknown", detail: "Checking for agent updates…" };
  if (query.error || !status) {
    return { kind: "unknown", detail: "Could not check — the hyprlayer CLI did not answer." };
  }
  if (!status.manageable) {
    return { kind: "unknown", detail: "The Claude and Codex bundle pair is not ready." };
  }
  const lastCheckedMs = status.lastChecked != null ? status.lastChecked * 1000 : null;
  const from =
    status.installedVersion ?? (status.installedSha ? shortSha(status.installedSha) : null);
  const to = status.desiredVersion ?? (status.latestSha ? shortSha(status.latestSha) : null);
  if (!status.updateAvailable || to === null) {
    return { kind: "upToDate", label: from, lastCheckedMs, frozen: status.desiredVersion === null };
  }
  return { kind: "updateAvailable", from, to };
}
