import { basename } from "@/lib/path";

export type PlanDocTask = { key: string; title: string; checked: boolean; line: number; explicitId: boolean };
export type CheckGroup = "automated" | "manual";
export type PlanDocCriterion = PlanDocTask & { group: CheckGroup | null };
export type PlanDocPhase = { label: string; name: string; tasks: PlanDocTask[]; criteria: PlanDocCriterion[] };
export type PlanDoc = {
  title: string;
  status: string | null;
  author: string | null;
  overview: string;
  phases: PlanDocPhase[];
  // Everything after the frontmatter and title; what a plan without phases renders.
  body: string;
};

const FRONTMATTER = /^---\r?\n(?:([\s\S]*?)\r?\n)?---(?:\r?\n|$)/;
const HEADING = /^(#{1,6})\s+(.*?)\s*#*\s*$/;
const PHASE = /^(?:.*?\s)?Phase\s+(\d+[a-z]?)\b\s*(?:[:—–-]\s*)?(.*)$/i;
const CHECKBOX = /^(\s*[-*+]\s+\[)([ xX~])(\]\s+)(.*)$/;
const FENCE = /^\s*(```|~~~)/;
const LEAD_ID = /^\[(P\d+[a-z]?\.\d+)\]\s*/i;
const TRAIL_ID = /<!--\s*id:\s*([\w.-]+)\s*-->/i;
const CRITERIA = /success criteria/i;

export function isDraftStatus(status: string | null): boolean {
  return !status || status.toLowerCase() === "draft";
}

function frontmatterValue(fm: string, key: string): string | null {
  const m = new RegExp(`^${key}:\\s*(.*)$`, "m").exec(fm);
  const raw = m?.[1].trim().replace(/^(["'])(.*)\1$/, "$2") ?? "";
  return raw === "" || raw === "null" || raw === "~" ? null : raw;
}

function cleanTitle(text: string): string {
  return text.replace(LEAD_ID, "").replace(/<!--.*?-->/g, "").replace(/`/g, "").trim();
}

function explicitTaskId(text: string): string | null {
  return LEAD_ID.exec(text)?.[1] ?? TRAIL_ID.exec(text)?.[1] ?? null;
}

// Tasks and checks in document order: keys, toggles and the tracker's rows all index this list.
export function phaseRows(phase: PlanDocPhase): (PlanDocTask | PlanDocCriterion)[] {
  return [...phase.tasks, ...phase.criteria].sort((a, b) => a.line - b.line);
}

function checkGroup(heading: string): CheckGroup | null {
  if (/automated/i.test(heading)) return "automated";
  return /manual/i.test(heading) ? "manual" : null;
}

// Explicit ids survive edits; positional keys are the fallback and never collide with them.
function assignKeys(phases: PlanDocPhase[]): void {
  const taken = new Set<string>();
  const pending: [PlanDocPhase, PlanDocTask, number][] = [];
  for (const phase of phases) {
    phaseRows(phase).forEach((task, i) => {
      if (task.explicitId && !taken.has(task.key)) taken.add(task.key);
      else pending.push([phase, task, i]);
    });
  }
  for (const [phase, task, i] of pending) {
    let key = `P${phase.label}.${i + 1}`;
    for (let n = 2; taken.has(key); n++) key = `P${phase.label}.${i + 1}~${n}`;
    task.key = key;
    task.explicitId = false;
    taken.add(key);
  }
}

export function parsePlanDoc(content: string, path: string): PlanDoc {
  const fmMatch = FRONTMATTER.exec(content);
  const fm = fmMatch?.[1] ?? "";
  const lines = content.split("\n").map((l) => l.replace(/\r$/, ""));
  const bodyStart = fmMatch ? fmMatch[0].split("\n").length - (fmMatch[0].endsWith("\n") ? 1 : 0) : 0;

  let h1: string | null = null;
  let h1Line = -1;
  const phases: PlanDocPhase[] = [];
  let phase: { level: number; phase: PlanDocPhase } | null = null;
  let criteria: { level: number; group: CheckGroup | null } | null = null;
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
        const next = { label: phaseMatch[1], name: phaseMatch[2].trim(), tasks: [], criteria: [] };
        phases.push(next);
        phase = { level, phase: next };
        criteria = null;
        continue;
      }
      if (phase && level <= phase.level) phase = null;
      if (criteria && (!phase || level <= criteria.level)) criteria = null;
      if (phase && CRITERIA.test(text)) criteria = { level, group: null };
      else if (criteria) criteria.group = checkGroup(text);
      if (level === 1 && h1 === null) {
        h1 = text;
        h1Line = i;
      }
      if (/^overview$/i.test(text) && !overviewText) overview = { level, lines: [] };
      continue;
    }
    overview?.lines.push(line);
    if (inFence || !phase) continue;
    const box = CHECKBOX.exec(line);
    if (!box) continue;
    const id = explicitTaskId(box[4]);
    const task = {
      key: id ?? "",
      title: cleanTitle(box[4]),
      checked: box[2] === "x" || box[2] === "X",
      line: i,
      explicitId: id !== null,
    };
    if (criteria) phase.phase.criteria.push({ ...task, group: criteria.group });
    else phase.phase.tasks.push(task);
  }
  if (overview) overviewText = overview.lines.join("\n").trim();
  assignKeys(phases);

  const body = lines
    .slice(bodyStart)
    .filter((_, i) => i + bodyStart !== h1Line)
    .join("\n")
    .trim();

  return {
    title: h1 ?? frontmatterValue(fm, "title") ?? (basename(path).replace(/\.md$/, "") || "Plan"),
    status: frontmatterValue(fm, "status"),
    author: frontmatterValue(fm, "author"),
    overview: overviewText,
    phases,
    body,
  };
}

// Finds the task by explicit id, then key+title, then title after an edit shifted keys; never guesses past that.
export function toggleTask(
  content: string,
  task: { key: string; title: string },
  checked: boolean,
): string | null {
  const all = parsePlanDoc(content, "").phases.flatMap(phaseRows);
  const target =
    all.find((t) => t.explicitId && t.key === task.key) ??
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
