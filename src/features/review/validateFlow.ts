import { useEffect, useMemo, useState } from "react";

import type { Session, WorkstreamView } from "@/lib/bindings";
import { permissionModeOption } from "@/lib/domain/permissions";
import { errorMessage } from "@/lib/ipc";
import { useValidationSummary, useWorkstreamSessions } from "@/lib/queries";
import { useToast } from "@/lib/toast";
import type { PlanTrackerResult } from "@/features/plan/usePlanTracker";
import {
  FIX_VALIDATION_PROMPT,
  restoreSessionIfNeeded,
  sendToImplementSession,
  useSession,
  validatePlanInNewSession,
  validatePlanInSession,
} from "@/features/session/sessionStore";
import { isBusy } from "@/features/session/transcript";
import { sessionPermissionMode } from "@/features/session/workstreamTabs";
import { useWorkstreamGitActions } from "@/features/workstreams/useGitActions";

import { findValidationReport } from "./validationReport";
import { useValidateStep, useValidateStepStore } from "./validateStep";

export type ValidatePhase =
  | "summarizing"
  | "waiting"
  | "unavailable"
  | "choosing"
  | "validating"
  | "issues"
  | "fixing"
  | "skipped"
  | "done";

const INLINE_GRACE_MS = 5_000;

// Shared by the chat card and the sidebar, so one click can't start two runs.
const starting = new Set<string>();

// Before a fresh session's entry loads, the server's flag stands in.
function useSessionBusy(workstreamId: string, sessions: Session[] | undefined, id: string | null): boolean {
  const entry = useSession(id ?? undefined);
  if (!id) return false;
  if (entry) return entry.workstreamId === workstreamId && isBusy(entry);
  return sessions?.find((s) => s.id === id)?.working ?? false;
}

// The report the validation ended with: from its own session, or after the prompt in the implement chat.
function useValidationSnapshot(workstreamId: string, implementId: string, agentId: string | null) {
  useEffect(() => {
    if (!agentId) return;
    const mode = permissionModeOption(sessionPermissionMode(workstreamId, agentId)).flag;
    void restoreSessionIfNeeded(agentId, workstreamId, "validate", mode);
  }, [agentId, workstreamId]);
  const sessionId = agentId ?? implementId;
  const items = useSession(sessionId)?.items;
  const found = useMemo(() => (items ? findValidationReport(items, agentId === null) : null), [items, agentId]);
  const cached = useValidateStepStore((s) => (found ? s.summaries[found.itemId] : undefined));
  const saveSummary = useValidateStepStore((s) => s.saveSummary);
  const summary = useValidationSummary(found && !cached ? found.itemId : null, found?.text ?? null);
  useEffect(() => {
    if (found && summary.data) saveSummary(found.itemId, summary.data);
  }, [found, summary.data, saveSummary]);
  // A failed or unusable summary falls back to the parse; only a pending one holds the step.
  const pending = found !== null && !cached && summary.isPending;
  const judged = cached ?? summary.data ?? null;
  const read = judged ?? found?.parsed;
  const report =
    found && read
      ? {
          itemId: found.itemId,
          judged: judged !== null,
          gate: read.gate ?? null,
          checks: read.checks ?? [],
          issues: read.issues ?? [],
          notes: read.notes ?? [],
          manual: read.manual ?? [],
        }
      : null;
  return { reportSessionId: sessionId, report, pending };
}

function fixPrompt(findings: string[]): string {
  return [
    "Resolve these findings from the plan validation. Fix the real ones, and say which you left alone and why.",
    "",
    ...findings.map((f) => `- ${f}`),
  ].join("\n");
}

export function useValidateFlow(
  view: WorkstreamView,
  implementId: string,
  plan: Pick<PlanTrackerResult, "tracker" | "key" | "inline">,
) {
  const workstreamId = view.item.id;
  const git = useWorkstreamGitActions(view);
  const sessions = useWorkstreamSessions(workstreamId).data;
  const step = useValidateStep(implementId);
  const setStep = useValidateStepStore((s) => s.setStep);
  const [busy, setBusy] = useState<"agent" | "inline" | "fix" | null>(null);
  const validateId = step?.kind === "agent" ? step.sessionId : null;
  const agentRunning = useSessionBusy(workstreamId, sessions, validateId);
  const ran = step?.kind === "agent" || step?.kind === "inline";
  const snapshot = useValidationSnapshot(workstreamId, implementId, ran ? validateId : null);
  const report = ran ? snapshot.report : null;
  const reportSessionId = snapshot.reportSessionId;
  // The summarizer already sorted pre-existing failures into notes; the bare parse can't tell.
  const findings = useMemo(() => {
    if (!report) return [];
    if (report.judged) return report.issues;
    return [...report.issues, ...report.checks.filter((c) => c.outcome === "fail").map((c) => `Failed check: ${c.text}`)];
  }, [report]);

  // Busy, not just working: a turn held on a permission prompt hasn't ended.
  const implementBusy = useSessionBusy(workstreamId, sessions, implementId);
  const built = (plan.tracker?.tasksDone ?? false) && !git.implementWorking && !implementBusy;
  const planPath = plan.key && !plan.inline ? plan.key : null;
  const passed = plan.tracker?.status === "implemented";
  const accepted = (step?.kind === "agent" || step?.kind === "inline") && step.accepted === true;
  const settled = ran && snapshot.pending ? "summarizing" : findings.length > 0 && !accepted ? "issues" : "done";
  // A settled step stays settled, so a later review round in the implement chat doesn't reopen it.
  const phase: ValidatePhase = (() => {
    switch (step?.kind) {
      case "skipped":
        return "skipped";
      case "fixing":
        return !step.finished ? "fixing" : planPath ? "choosing" : "unavailable";
      case "inline":
        return step.finished ? settled : "validating";
      case "agent":
        return agentRunning ? "validating" : settled;
      default:
        return passed ? "done" : !built ? "waiting" : !planPath ? "unavailable" : "choosing";
    }
  })();

  // The send may land a beat before the turn shows as working, so idle right after it isn't the end.
  const turnSettled =
    (step?.kind === "inline" || step?.kind === "fixing") &&
    !step.finished &&
    built &&
    Date.now() - (step.sentAt ?? 0) > INLINE_GRACE_MS;
  useEffect(() => {
    if (turnSettled) setStep(implementId, { ...step, finished: true });
    // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed on the settle flag
  }, [turnSettled, implementId]);

  const implement = sessions?.find((s) => s.id === implementId) ?? null;
  const validateSession = sessions?.find((s) => s.id === validateId) ?? null;

  const guarded = async (kind: "agent" | "inline" | "fix", title: string, work: () => Promise<void>) => {
    if (starting.has(implementId)) return;
    starting.add(implementId);
    setBusy(kind);
    try {
      await work();
    } catch (e) {
      useToast.getState().show(title, errorMessage(e), { tone: "bad" });
    } finally {
      starting.delete(implementId);
      setBusy(null);
    }
  };

  const run = (how: "agent" | "inline") =>
    guarded(how, "Validation didn't start", async () => {
      if (!planPath || !implement) return;
      if (how === "inline") {
        const outcome = await validatePlanInSession(workstreamId, implement, planPath);
        if (outcome === "dropped") throw new Error("The implementation chat couldn't be resumed.");
        setStep(implementId, { kind: "inline", sentAt: Date.now() });
      } else {
        const session = await validatePlanInNewSession(workstreamId, planPath, implementId);
        setStep(implementId, { kind: "agent", sessionId: session.id });
      }
    });

  const fix = (only?: string) =>
    guarded("fix", "Fix didn't start", async () => {
      if (!implement) return;
      const text = fixPrompt(only ? [only] : findings);
      const outcome = await sendToImplementSession(workstreamId, implement, text, FIX_VALIDATION_PROMPT);
      if (outcome === "dropped") throw new Error("The implementation chat couldn't be resumed.");
      setStep(implementId, { kind: "fixing", sentAt: Date.now() });
    });

  return {
    phase,
    built,
    passed,
    accepted,
    git,
    step,
    busy,
    canStart: implement !== null && busy === null,
    implement,
    validateSession,
    report,
    reportSessionId,
    findings,
    run,
    fix,
    accept: () => {
      if (step?.kind === "agent" || step?.kind === "inline") setStep(implementId, { ...step, accepted: true });
    },
    skip: () => setStep(implementId, { kind: "skipped" }),
  };
}

export type ValidateFlow = ReturnType<typeof useValidateFlow>;
