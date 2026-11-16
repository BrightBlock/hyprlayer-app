import { create } from "zustand";
import { persist } from "zustand/middleware";

import type { Session, ValidationSummary } from "@/lib/bindings";
import { scopedKey } from "@/lib/hostScope";
import { useWorkstreamSessions } from "@/lib/queries";
import { FIX_VALIDATION_PROMPT } from "@/features/session/sessionStore";
import type { TranscriptItem } from "@/features/session/transcript";

// How the validate step after an implement session was settled; `agent` names the validate session.
// `fixing` sends the report's findings back to the implement chat; once that turn ends, validation is offered again.
export type ValidateStep =
  | { kind: "skipped" }
  | { kind: "inline"; sentAt?: number; finished?: boolean; accepted?: boolean }
  | { kind: "agent"; sessionId: string; accepted?: boolean }
  | { kind: "fixing"; sentAt: number; finished?: boolean };

type ValidateStepStore = {
  byImplement: Record<string, ValidateStep>;
  // Keyed by the report's transcript item; a report never changes, so neither does its summary.
  summaries: Record<string, ValidationSummary>;
  setStep: (implementSessionId: string, step: ValidateStep) => void;
  saveSummary: (reportId: string, summary: ValidationSummary) => void;
};

export const useValidateStepStore = create<ValidateStepStore>()(
  persist(
    (set) => ({
      byImplement: {},
      summaries: {},
      setStep: (implementSessionId, step) =>
        set((s) => ({ byImplement: { ...s.byImplement, [implementSessionId]: step } })),
      saveSummary: (reportId, summary) => set((s) => ({ summaries: { ...s.summaries, [reportId]: summary } })),
    }),
    { name: scopedKey("hyprlayer:validate-steps"), version: 1 },
  ),
);

export function useValidateStep(implementSessionId: string | null): ValidateStep | null {
  return useValidateStepStore((s) => (implementSessionId ? (s.byImplement[implementSessionId] ?? null) : null));
}

export function useImplementSessionFor(workstreamId: string, validateSessionId: string): string | null {
  const local = useValidateStepStore((s) => {
    for (const [implementId, step] of Object.entries(s.byImplement)) {
      if (step.kind === "agent" && step.sessionId === validateSessionId) return implementId;
    }
    return null;
  });
  const row = useWorkstreamSessions(workstreamId, local === null).data?.find((s) => s.id === validateSessionId);
  return local ?? (row?.stage === "validate" ? (row.parentSessionId ?? null) : null);
}

/**
 * The step another desktop started, read from the validate session's link to `implementId`.
 * Only a fix prompt sent after the validation started counts as fixing; ordinary chat doesn't.
 * Without the implement chat's items loaded, the validation itself is the step.
 */
export function linkedValidateStep(
  sessions: Session[] | undefined,
  implementId: string,
  implementItems: readonly TranscriptItem[] | undefined,
  implementBusy: boolean,
): ValidateStep | null {
  const validate = sessions
    ?.filter((s) => s.stage === "validate" && s.parentSessionId === implementId && s.archivedAt == null)
    .sort((a, b) => (b.startedAt ?? 0) - (a.startedAt ?? 0))[0];
  if (!validate) return null;
  const agent: ValidateStep = { kind: "agent", sessionId: validate.id };
  const since = validate.startedAt;
  if (validate.working || !implementItems || since == null) return agent;
  let laterTurn = false;
  for (let i = implementItems.length - 1; i >= 0; i--) {
    const item = implementItems[i];
    if (item.kind !== "user") continue;
    // A live echo has no `createdAt` until it is patched in; it is newer than any finished validation.
    if (item.createdAt == null) {
      laterTurn = true;
      continue;
    }
    if (item.createdAt <= since) break;
    if (item.promptName === FIX_VALIDATION_PROMPT) {
      return { kind: "fixing", sentAt: item.createdAt, finished: laterTurn || !implementBusy };
    }
    laterTurn = true;
  }
  return agent;
}
