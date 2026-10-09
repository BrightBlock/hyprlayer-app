import { create } from "zustand";
import { persist } from "zustand/middleware";

import type { ValidationSummary } from "@/lib/bindings";
import { scopedKey } from "@/lib/hostScope";

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

export function useImplementSessionFor(validateSessionId: string): string | null {
  return useValidateStepStore((s) => {
    for (const [implementId, step] of Object.entries(s.byImplement)) {
      if (step.kind === "agent" && step.sessionId === validateSessionId) return implementId;
    }
    return null;
  });
}
