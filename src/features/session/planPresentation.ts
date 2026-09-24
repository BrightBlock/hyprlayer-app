import { referencedPlan } from "./planLink";
import { canonicalDocPath, docFromTool, parseToolInput } from "./toolDisplay";
import type { TranscriptItem } from "./transcript";
import type { WrittenDocs } from "./writtenDocs";

// Every plan that exists only as text in the transcript (codex plan mode, a fileless ExitPlanMode) shares one key.
export const INLINE_PLAN_KEY = "inline";

export type PlanPresentationTool = "ExitPlanMode" | "ProposedPlan" | "plan_exit";

// A harness handing its plan to the user: Claude's ExitPlanMode, codex's proposed plan, OpenCode's plan_exit.
export type PlanPresentation = {
  toolUseId: string;
  tool: PlanPresentationTool;
  key: string;
  text: string | null;
  state: "pending" | "proposed" | "approved" | "rejected";
  requestId: string | null;
};

const PRESENTATION_TOOLS = new Set<string>(["ExitPlanMode", "ProposedPlan", "plan_exit"]);

// Narrow on purpose: anything conversational ("approved, but…") still reaches the agent.
const APPROVAL = /^(?:approved?(?:\s*(?:[.,!]|and)\s*build(?:\s+it)?)?|lgtm[,.!]?\s+build\s+it)[.!]?$/i;

export function isApprovalMessage(text: string): boolean {
  return APPROVAL.test(text.trim());
}

export function isInlinePlanKey(key: string): boolean {
  return key === INLINE_PLAN_KEY;
}

function stringField(input: string, field: string): string | null {
  const value = parseToolInput(input)?.[field];
  return typeof value === "string" && value.trim() ? value : null;
}

const cache = new WeakMap<TranscriptItem[], { written: WrittenDocs; presentations: PlanPresentation[] }>();

export function planPresentations(items: TranscriptItem[], written: WrittenDocs): PlanPresentation[] {
  const hit = cache.get(items);
  if (hit && hit.written === written) return hit.presentations;

  const presentations: PlanPresentation[] = [];
  let lastWritten: string | null = null;
  for (const [i, item] of items.entries()) {
    if (item.kind === "user" && isApprovalMessage(item.text)) {
      for (const p of presentations) if (p.state === "proposed") p.state = "approved";
      continue;
    }
    if (item.kind !== "tool" || item.parentId) continue;
    if (!PRESENTATION_TOOLS.has(item.name)) {
      const doc = item.result && !item.result.isError ? docFromTool(item.name, item.input, written) : null;
      if (doc?.kind === "plan") lastWritten = doc.path;
      continue;
    }
    const tool = item.name as PlanPresentationTool;
    const text = stringField(item.input, "plan");
    const file = stringField(item.input, "planFilePath");
    const key = file ? canonicalDocPath(file, written) : tool === "ProposedPlan" ? null : lastWritten;
    if (!key && !text) continue;
    const state = !item.result
      ? "pending"
      : item.result.isError
        ? "rejected"
        : tool === "ProposedPlan"
          ? "proposed"
          : "approved";
    const permission =
      state === "pending"
        ? items.slice(i + 1).find((p) => p.kind === "permission" && p.toolName === tool && p.decision === null)
        : undefined;
    presentations.push({
      toolUseId: item.toolUseId,
      tool,
      key: key ?? INLINE_PLAN_KEY,
      text,
      state,
      requestId: permission?.kind === "permission" ? permission.requestId : null,
    });
  }
  cache.set(items, { written, presentations });
  return presentations;
}

export function presentationFor(items: TranscriptItem[], written: WrittenDocs, key: string): PlanPresentation | null {
  const all = planPresentations(items, written);
  for (let i = all.length - 1; i >= 0; i--) if (all[i].key === key) return all[i];
  return null;
}

// The newest plan the session wrote or presented; an implement session keeps the plan it was given.
export function planKeyForSession(items: TranscriptItem[], stage: string, written: WrittenDocs): string | null {
  const presented = new Map(planPresentations(items, written).map((p) => [p.toolUseId, p.key]));
  let newest: string | null = null;
  for (const item of items) {
    if (item.kind !== "tool" || item.parentId) continue;
    const key = presented.get(item.toolUseId);
    if (key) newest = key;
    else if (item.result && !item.result.isError) {
      const doc = docFromTool(item.name, item.input, written);
      if (doc?.kind === "plan") newest = doc.path;
    }
  }
  return stage === "implement" ? (referencedPlan(items) ?? newest) : (newest ?? referencedPlan(items));
}
