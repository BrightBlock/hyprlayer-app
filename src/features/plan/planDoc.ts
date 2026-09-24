import { basename } from "@/lib/path";

export type PlanDocTask = { key: string; title: string; checked: boolean; line: number };
export type PlanDocPhase = { label: string; name: string; tasks: PlanDocTask[] };
export type PlanDoc = {
  title: string;
  status: string | null;
  author: string | null;
  overview: string;
  phases: PlanDocPhase[];
};

const FRONTMATTER = /^---\r?\n(?:([\s\S]*?)\r?\n)?---(?:\r?\n|$)/;
const HEADING = /^(#{1,6})\s+(.*?)\s*#*\s*$/;
const PHASE = /^(?:.*?\s)?Phase\s+(\d+[a-z]?)\b\s*(?:[:—–-]\s*)?(.*)$/i;
const CHECKBOX = /^(\s*[-*+]\s+\[)([ xX~])(\]\s+)(.*)$/;
const FENCE = /^\s*(```|~~~)/;

function frontmatterValue(fm: string, key: string): string | null {
  const m = new RegExp(`^${key}:\\s*(.*)$`, "m").exec(fm);
  const raw = m?.[1].trim().replace(/^(["'])(.*)\1$/, "$2") ?? "";
  return raw === "" || raw === "null" || raw === "~" ? null : raw;
}

function cleanTitle(text: string): string {
  return text.replace(/<!--.*?-->/g, "").replace(/`/g, "").trim();
}

export function parsePlanDoc(content: string, path: string): PlanDoc {
  const fmMatch = FRONTMATTER.exec(content);
  const fm = fmMatch?.[1] ?? "";
  const lines = content.split("\n").map((l) => l.replace(/\r$/, ""));
  const bodyStart = fmMatch ? fmMatch[0].split("\n").length - (fmMatch[0].endsWith("\n") ? 1 : 0) : 0;

  let h1: string | null = null;
  const phases: PlanDocPhase[] = [];
  let phase: { level: number; phase: PlanDocPhase } | null = null;
  let overview: { level: number; lines: string[] } | null = null;
  let overviewText = "";
  let inFence = false;

  for (let i = bodyStart; i < lines.length; i++) {
    const line = lines[i];
    if (FENCE.test(line)) {
      inFence = !inFence;
      overview?.lines.push(line);
      continue;
    }
    const heading = inFence ? null : HEADING.exec(line);
    if (heading) {
      const level = heading[1].length;
      const text = heading[2];
      if (overview && level <= overview.level) {
        overviewText = overview.lines.join("\n").trim();
        overview = null;
      }
      const phaseMatch = level <= 3 ? PHASE.exec(text) : null;
      if (phaseMatch) {
        const next = { label: phaseMatch[1], name: phaseMatch[2].trim(), tasks: [] };
        phases.push(next);
        phase = { level, phase: next };
        continue;
      }
      if (phase && level <= phase.level) phase = null;
      if (level === 1 && h1 === null) h1 = text;
      if (/^overview$/i.test(text) && !overviewText) overview = { level, lines: [] };
      continue;
    }
    overview?.lines.push(line);
    if (inFence || !phase) continue;
    const box = CHECKBOX.exec(line);
    if (!box) continue;
    const tasks = phase.phase.tasks;
    tasks.push({
      key: `P${phase.phase.label}.${tasks.length + 1}`,
      title: cleanTitle(box[4]),
      checked: box[2] === "x" || box[2] === "X",
      line: i,
    });
  }
  if (overview) overviewText = overview.lines.join("\n").trim();

  return {
    title: h1 ?? frontmatterValue(fm, "title") ?? basename(path).replace(/\.md$/, ""),
    status: frontmatterValue(fm, "status"),
    author: frontmatterValue(fm, "author"),
    overview: overviewText,
    phases,
  };
}

// Finds the task by key, else by title after an edit shifted keys; never guesses past that.
export function toggleTask(
  content: string,
  task: { key: string; title: string },
  checked: boolean,
): string | null {
  const all = parsePlanDoc(content, "").phases.flatMap((p) => p.tasks);
  const target =
    all.find((t) => t.key === task.key && t.title === task.title) ??
    all.find((t) => t.title === task.title);
  if (!target) return null;
  if (target.checked === checked) return content;
  const lines = content.split("\n");
  const raw = lines[target.line];
  const cr = raw.endsWith("\r") ? "\r" : "";
  lines[target.line] =
    raw
      .slice(0, raw.length - cr.length)
      .replace(CHECKBOX, (_, pre, _mark, post, rest) => `${pre}${checked ? "x" : " "}${post}${rest}`) + cr;
  return lines.join("\n");
}
