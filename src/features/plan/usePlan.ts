import { useEffect, useMemo, useRef, useState } from "react";

import { isInlinePlanKey } from "@/features/session/planPresentation";
import { useDoc } from "@/lib/queries";

import { parsePlanDoc, type PlanDoc } from "./planDoc";

export const FLASH_MS = 1000;

const NO_CHANGES: ReadonlySet<string> = new Set();

export function planSections(plan: PlanDoc): Map<string, string> {
  const sections = new Map<string, string>([
    ["header", JSON.stringify([plan.title, plan.status, plan.author])],
    ["overview", plan.overview],
  ]);
  if (plan.phases.length === 0) sections.set("body", plan.body);
  for (const p of plan.phases) {
    sections.set(`phase:${p.label}`, JSON.stringify([p.name, p.tasks.map((t) => [t.key, t.title, t.checked])]));
  }
  return sections;
}

// Sections whose content differs from the previous parse of the same doc, cleared after FLASH_MS.
function useChangedSections(path: string | null, plan: PlanDoc | null): ReadonlySet<string> {
  const previous = useRef<{ path: string | null; sections: Map<string, string> } | null>(null);
  const [changed, setChanged] = useState<ReadonlySet<string>>(NO_CHANGES);

  useEffect(() => {
    if (!plan) return;
    const sections = planSections(plan);
    const prior = previous.current;
    previous.current = { path, sections };
    if (!prior || prior.path !== path) return;
    const diff = new Set([...sections].filter(([k, v]) => prior.sections.get(k) !== v).map(([k]) => k));
    if (diff.size > 0) setChanged(diff);
  }, [path, plan]);

  useEffect(() => {
    if (changed.size === 0) return;
    const timer = setTimeout(() => setChanged(NO_CHANGES), FLASH_MS);
    return () => clearTimeout(timer);
  }, [changed]);

  return changed;
}

// `key` is a plan doc path, or the inline key with the plan's text passed alongside.
export function usePlan(key: string | null, inlineText: string | null = null) {
  const inline = key !== null && isInlinePlanKey(key);
  const doc = useDoc(inline || !key ? "" : key);
  const content = inline ? (inlineText ?? undefined) : doc.data?.content;
  const plan = useMemo(
    () => (content !== undefined && key ? parsePlanDoc(content, inline ? "" : key) : null),
    [content, key, inline],
  );
  const changed = useChangedSections(key, plan);
  return {
    plan,
    inline,
    // Only an indexed thoughts doc may be written; `unindexed_meta` leaves `backend` empty.
    indexed: !inline && doc.data !== undefined && doc.data.meta.backend !== "",
    mtimeMs: inline ? null : (doc.data?.mtimeMs ?? null),
    loading: !inline && Boolean(key) && doc.isLoading,
    error: inline ? null : doc.error,
    refetch: doc.refetch,
    changed,
  };
}
