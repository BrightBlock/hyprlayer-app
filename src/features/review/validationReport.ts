import type { CheckOutcome, ValidationSummary } from "@/lib/bindings";
import { VALIDATE_PROMPT } from "@/features/session/sessionStore";
import type { TranscriptItem } from "@/features/session/transcript";

// `parsed` is the instant fallback; the summarizer's reading replaces it when it lands.
export type FoundReport = { itemId: string; text: string; parsed: ValidationSummary };

type SectionKey = "status" | "checks" | "findings" | "matches" | "deviations" | "issues" | "manual" | "recs" | "gate";

const REPORT_HEADING = /^#{1,4}\s*validation report\b/im;
const HEADING = /^(?:#{2,5}\s+(.+?)|\*\*(.+?):?\*\*:?\s*(.*))$/;
const BULLET = /^(\s*)(?:[-*]|\d+\.)\s+(?:\[[ xX]\]\s+)?(.*)$/;
const OUTCOME_LABEL = /^(pass(ed)?|fail(ed)?)\s*:\s*/i;
const NONE = /^(none|n\/a|no (deviations|issues|manual))\b/i;

function sectionOf(title: string): SectionKey | null {
  const t = title.toLowerCase();
  if (t.includes("gate")) return "gate";
  if (t.includes("automated")) return "checks";
  if (t.includes("deviation")) return "deviations";
  if (t.includes("issue") || t.includes("risk")) return "issues";
  if (t.includes("matches")) return "matches";
  if (t.includes("manual")) return "manual";
  if (t.includes("recommend")) return "recs";
  if (t.includes("review finding") || t.includes("findings")) return "findings";
  if (t.includes("implementation")) return "status";
  return null;
}

function plain(text: string): string {
  return text
    .replace(/\*\*(.+?)\*\*/g, "$1")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/\s+/g, " ")
    .trim();
}

function outcomeOf(text: string): CheckOutcome | null {
  if (/\bfail(s|ed|ing)?\b/i.test(text)) return "fail";
  if (/\bpass(es|ed)?\b|\bexit(s|ed)? 0\b/i.test(text)) return "pass";
  return null;
}

// Top-level bullets, except a group label ending in `:` yields its nested items instead.
function items(lines: string[]): string[] {
  const out: string[] = [];
  let group: string | null = null;
  const prose: string[] = [];
  for (const line of lines) {
    const m = BULLET.exec(line);
    if (!m) {
      if (line.trim()) prose.push(line.trim());
      continue;
    }
    const nested = m[1].length > 0;
    const text = m[2].trim();
    if (!nested) {
      group = plain(text).endsWith(":") ? text : null;
      if (!group) out.push(text);
    } else if (group) {
      out.push(text);
    }
  }
  if (out.length === 0 && prose.length > 0) out.push(prose.join(" "));
  return out.map(plain).filter((t) => t && !NONE.test(t));
}

function reportStart(text: string): number {
  return text.search(REPORT_HEADING);
}

export function parseValidationReport(text: string): ValidationSummary {
  const start = Math.max(0, reportStart(text));
  const sections = new Map<SectionKey, string[]>();
  let current: SectionKey | null = null;
  for (const line of text.slice(start).split("\n").slice(1)) {
    const h = HEADING.exec(line.trim());
    if (h) {
      const key = sectionOf(h[1] ?? h[2]);
      if (key) {
        current = key;
        const rest = h[3]?.trim();
        sections.set(key, rest ? [rest] : []);
        continue;
      }
    }
    if (current) sections.get(current)!.push(line);
  }
  const of = (key: SectionKey) => items(sections.get(key) ?? []);
  const gate = sections.get("gate")?.map((l) => l.trim()).find(Boolean);

  return {
    gate: gate ? plain(gate) : null,
    checks: of("checks").map((t) => ({ text: t.replace(OUTCOME_LABEL, ""), outcome: outcomeOf(t) })),
    issues: [...of("deviations"), ...of("issues")],
    notes: [],
    manual: of("manual"),
  };
}

// The newest report among the session's own replies, after the validate prompt when it was sent inline.
export function findValidationReport(items: readonly TranscriptItem[], afterPrompt: boolean): FoundReport | null {
  let from = 0;
  if (afterPrompt) {
    for (let i = items.length - 1; i >= 0; i--) {
      const it = items[i];
      if (it.kind === "user" && it.promptName === VALIDATE_PROMPT) {
        from = i + 1;
        break;
      }
    }
    if (from === 0) return null;
  }
  for (let i = items.length - 1; i >= from; i--) {
    const it = items[i];
    if (it.kind !== "assistant" || it.parentId || it.streaming) continue;
    const start = reportStart(it.text);
    if (start < 0) continue;
    const text = it.text.slice(start);
    return { itemId: it.id, text, parsed: parseValidationReport(text) };
  }
  return null;
}
