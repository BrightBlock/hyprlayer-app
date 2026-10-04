import type { AgentFile, SkillFile } from "@/lib/bindings";
import { formatBytes, formatCompactCount, plural } from "@/lib/format";
import { basename } from "@/lib/path";

export type SkillFilter = "all" | "yours" | "bundle" | "plugins";

export const SKILL_FILTERS: ReadonlyArray<{ key: SkillFilter; label: string }> = [
  { key: "all", label: "All" },
  { key: "yours", label: "Yours" },
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
      return skills.filter((s) => !s.managed && s.source !== "plugin");
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
  if (s.source === "project") return `${projectName ?? "project"}/.claude/skills/${dir}`;
  if (s.source === "user") return `~/.claude/skills/${dir}`;
  return `${s.plugin ?? "plugin"} · skills/${dir}`;
}

export function attachedAgents(skill: SkillFile, agents: readonly AgentFile[]): AgentFile[] {
  return agents.filter((a) => a.skills.includes(skill.agentRef));
}

// Anthropic's guidance keeps a SKILL.md body under 5k tokens; ~4 chars per token.
export const SKILL_TOKEN_BUDGET = 5000;

export function skillTokenLine(s: SkillFile): string {
  return `~${formatCompactCount(s.chars / 4)} / ${formatCompactCount(SKILL_TOKEN_BUDGET)} tokens`;
}

export function skillMeta(s: SkillFile): string {
  return `SKILL.md ${formatBytes(s.bytes ?? 0)} · ${plural(s.assets.length, "file")} · /${s.slashName}`;
}

export type SkillTab = "source" | "setup" | "tools" | "files";

export const SKILL_TABS: ReadonlyArray<{ key: SkillTab; label: string }> = [
  { key: "source", label: "SKILL.md" },
  { key: "setup", label: "Setup" },
  { key: "tools", label: "Tools & agents" },
  { key: "files", label: "Files" },
];

export function skillTab(raw: string | undefined): SkillTab {
  return SKILL_TABS.find((t) => t.key === raw)?.key ?? "source";
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

export const MODELLED_KEYS = ["name", "description", "allowed-tools", "model", "argument-hint", "disable-model-invocation"];

export type SkillFileRow = {
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
