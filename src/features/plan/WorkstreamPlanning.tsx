import type { ReactNode } from "react";

import { isInlinePlanKey, planKeyForSession, presentationFor } from "@/features/session/planPresentation";
import { useSessionStore } from "@/features/session/sessionStore";
import { isTerminal } from "@/features/session/transcript";
import { useWrittenDocs } from "@/features/session/writtenDocs";

import { isDraftStatus } from "./planDoc";
import { usePlan } from "./usePlan";

// The plan a running, non-implement session in this workstream wrote, presented or was given, with its
// presentation state; joined into one string so the selector stays referentially stable.
function useWorkstreamPlan(workstreamId: string): { key: string | null; state: string | null } {
  const written = useWrittenDocs(workstreamId);
  const joined = useSessionStore((s) => {
    let found = "";
    for (const entry of Object.values(s.sessions)) {
      if (entry.workstreamId !== workstreamId || entry.stage === "implement" || isTerminal(entry.state)) continue;
      const key = planKeyForSession(entry.items, entry.stage, written);
      if (key) found = `${key}\t${presentationFor(entry.items, written, key)?.state ?? ""}`;
    }
    return found;
  });
  if (!joined) return { key: null, state: null };
  const [key, state] = joined.split("\t");
  return { key, state: state || null };
}

export function WorkstreamPlanning({ workstreamId, fallback }: { workstreamId: string; fallback: ReactNode }) {
  const { key, state } = useWorkstreamPlan(workstreamId);
  const { plan } = usePlan(key && !state && !isInlinePlanKey(key) ? key : null);
  const planning = state ? state !== "approved" : plan !== null && isDraftStatus(plan.status);
  if (!planning) return fallback;
  return <span className="shrink-0 text-meta tabular-nums text-ink-4">planning</span>;
}
