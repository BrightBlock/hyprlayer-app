import type { AgentFile, SkillFile, SkillInstallPreview, SkillScope, SkillsShView, SkillTarget } from "@/lib/bindings";
import { formatBytes, formatCompactCount, plural, shortSha } from "@/lib/format";
import { basename, dirname } from "@/lib/path";

export type SkillFilter = "all" | "yours" | "installed" | "bundle" | "plugins";

export const SKILL_FILTERS: ReadonlyArray<{ key: SkillFilter; label: string }> = [
  { key: "all", label: "All" },
  { key: "yours", label: "Yours" },
  { key: "installed", label: "Installed" },
  { key: "bundle", label: "Hyprlayer bundle" },
  { key: "plugins", label: "Plugins" },
];

export const MAX_SKILL_ASSETS = 200;

export function skillFilter(raw: string | undefined): SkillFilter {
  return SKILL_FILTERS.find((f) => f.key === raw)?.key ?? "all";
}

export function filterSkills(skills: readonly SkillFile[], filter: SkillFilter): SkillFile[] {
  switch (filter) {
    case "yours":
      return skills.filter((s) => !s.managed && !s.installed && s.source !== "plugin");
    case "installed":
      return skills.filter((s) => s.installed);
    case "bundle":
      return skills.filter((s) => s.managed);
    case "plugins":
      return skills.filter((s) => s.source === "plugin");
    default:
      return [...skills];
  }
}

// Bundle skills live under the user root, so `managed` wins over `source`.
export function skillSourceLabel(s: SkillFile): string {
  if (s.managed) return "Hyprlayer";
  if (s.source === "project") return "Project";
  if (s.source === "user") return "Personal";
  return s.plugin ?? "Plugin";
}

export function skillLocation(s: SkillFile, projectName: string | null): string {
  const dir = basename(s.dir);
  const root = basename(dirname(dirname(s.dir))) === ".agents" ? ".agents/skills" : ".claude/skills";
  if (s.source === "project") return `${projectName ?? "project"}/${root}/${dir}`;
  // A CLAUDE_CONFIG_DIR user root shows its real path.
  if (s.source === "user") return root === ".agents/skills" || /[/\\]\.claude[/\\]skills[/\\]/.test(s.dir) ? `~/${root}/${dir}` : s.dir;
  return `${s.plugin ?? "plugin"} · skills/${dir}`;
}

export const SKILLS_SH_VIEWS: ReadonlyArray<{ value: SkillsShView; label: string }> = [
  { value: "all-time", label: "All time" },
  { value: "trending", label: "Trending" },
  { value: "hot", label: "Hot" },
];

export const SKILL_TARGETS: readonly SkillTarget[] = ["claude", "agents"];
export const SKILL_TARGET_LABEL: Record<SkillTarget, string> = { claude: "Claude Code", agents: "Codex, OpenCode & others" };
export const SKILL_TARGET_BADGE: Record<SkillTarget, string> = { claude: "Claude Code", agents: "Codex & others" };

// The lock and remove/update key; the frontmatter name can differ.
export function skillFolder(s: SkillFile): string {
  return basename(s.dir);
}

export function skillOriginLine(s: SkillFile): string | null {
  return s.origin ? `${s.origin.repo}@${s.origin.ref} · ${shortSha(s.origin.commit)}` : null;
}

export function skillScope(s: SkillFile): SkillScope {
  return s.source === "project" ? "project" : "global";
}

export function sourceLine(p: SkillInstallPreview): string {
  const base = `${p.repo}@${p.ref}${p.path ? ` · ${p.path}` : ""} · ${shortSha(p.commit)}`;
  return p.fetchedFrom ? `${base} · via ${p.fetchedFrom}` : base;
}

export function attachedAgents(skill: SkillFile, agents: readonly AgentFile[]): AgentFile[] {
  return agents.filter((a) => a.skills.includes(skill.agentRef));
}

// Anthropic's guidance keeps a SKILL.md body under 5k tokens; ~4 chars per token.
const SKILL_TOKEN_BUDGET = 5000;

export function skillTokenLine(s: SkillFile): string {
  return `~${formatCompactCount(s.chars / 4)} / ${formatCompactCount(SKILL_TOKEN_BUDGET)} tokens`;
}

export function skillMeta(s: SkillFile): string {
  return `SKILL.md ${formatBytes(s.bytes ?? 0)} · ${plural(s.assets.length, "file")} · /${s.slashName}`;
}

export type SkillTab = "instructions" | "files";

export const SKILL_TABS: readonly SkillTab[] = ["instructions", "files"];

export function skillTab(raw: string | undefined): SkillTab {
  return SKILL_TABS.find((t) => t === raw) ?? "instructions";
}

// The frontmatter is already shown as fields; the Instructions tab renders what follows it.
export function skillBody(text: string): string {
  const m = /^---\r?\n[\s\S]*?\r?\n---[ \t]*(?:\r?\n|$)/.exec(text);
  return m ? text.slice(m[0].length) : text;
}

export function frontmatterValue(s: SkillFile, key: string): string | null {
  const value = s.frontmatter.find((f) => f.key === key)?.value.trim();
  return value ? value : null;
}

// Core's display value: a YAML list arrives as JSON, a scalar as "a, b".
export function preapprovedTools(s: SkillFile): string[] {
  const raw = frontmatterValue(s, "allowed-tools");
  if (!raw) return [];
  if (raw.startsWith("[")) {
    try {
      const list: unknown = JSON.parse(raw);
      if (Array.isArray(list)) return list.map(String).filter(Boolean);
    } catch {
      // fall through to the comma split
    }
  }
  return raw.split(",").map((t) => t.trim()).filter(Boolean);
}

type SkillFileRow = {
  path: string;
  name: string;
  depth: number;
  dir: boolean;
  bytes: number | null;
  step: string;
};

export function skillFileRows(s: SkillFile): SkillFileRow[] {
  const rows: SkillFileRow[] = [
    { path: "SKILL.md", name: "SKILL.md", depth: 0, dir: false, bytes: s.bytes, step: "On trigger" },
  ];
  const seenDirs = new Set<string>();
  for (const a of s.assets) {
    const parts = a.relPath.split("/");
    for (let i = 0; i < parts.length - 1; i++) {
      const dir = parts.slice(0, i + 1).join("/");
      if (seenDirs.has(dir)) continue;
      seenDirs.add(dir);
      rows.push({ path: dir, name: parts[i], depth: i, dir: true, bytes: null, step: "" });
    }
    rows.push({
      path: a.relPath,
      name: parts[parts.length - 1],
      depth: parts.length - 1,
      dir: false,
      bytes: a.bytes,
      step: "On demand",
    });
  }
  return rows;
}
