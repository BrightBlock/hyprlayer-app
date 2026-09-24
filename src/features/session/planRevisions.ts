import { INLINE_PLAN_KEY, planPresentations } from "./planPresentation";
import { docFromTool } from "./toolDisplay";
import type { TranscriptItem } from "./transcript";
import type { WrittenDocs } from "./writtenDocs";

// `planKey` is a plan doc path or the inline key; `text` is the newest inline plan, which every inline card shows.
// `latest` marks the session's newest plan, the one the preview shows when no card was picked.
export type PlanRevision = { planKey: string; text: string | null; superseded: boolean; latest: boolean };

const cache = new WeakMap<TranscriptItem[], { written: WrittenDocs; revisions: Map<string, PlanRevision> }>();

// A revision is the last finished write or presentation of a plan within one user turn, keyed by toolUseId.
export function planRevisions(items: TranscriptItem[], written: WrittenDocs): Map<string, PlanRevision> {
  const hit = cache.get(items);
  if (hit && hit.written === written) return hit.revisions;

  const presented = new Map(planPresentations(items, written).map((p) => [p.toolUseId, p]));
  const ordered: { toolUseId: string; key: string }[] = [];
  let inlineText: string | null = null;
  let turn = new Map<string, number>();
  for (const item of items) {
    if (item.kind === "user") {
      turn = new Map();
      continue;
    }
    if (item.kind !== "tool" || item.parentId) continue;
    const presentation = presented.get(item.toolUseId);
    let key = presentation?.key ?? null;
    if (presentation?.key === INLINE_PLAN_KEY) inlineText = presentation.text;
    if (!key && item.result && !item.result.isError) {
      const doc = docFromTool(item.name, item.input, written);
      if (doc?.kind === "plan") key = doc.path;
    }
    if (!key) continue;
    const prior = turn.get(key);
    const entry = { toolUseId: item.toolUseId, key };
    if (prior === undefined) {
      turn.set(key, ordered.length);
      ordered.push(entry);
    } else {
      ordered[prior] = entry;
    }
  }

  const latest = new Map<string, string>();
  for (const r of ordered) latest.set(r.key, r.toolUseId);
  const newest = ordered[ordered.length - 1]?.toolUseId;
  const revisions = new Map<string, PlanRevision>(
    ordered.map((r) => [
      r.toolUseId,
      {
        planKey: r.key,
        text: r.key === INLINE_PLAN_KEY ? inlineText : null,
        superseded: latest.get(r.key) !== r.toolUseId,
        latest: r.toolUseId === newest,
      },
    ]),
  );
  cache.set(items, { written, revisions });
  return revisions;
}
