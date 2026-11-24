import {
  isApprovalMessage,
  isInlinePlanKey,
  planKeyForSession,
  presentationFor,
  type PlanPresentation,
} from "@/features/session/planPresentation";
import type { OutgoingMessage } from "@/features/session/sendInterceptor";
import { answerQuestion, respondPermission, sendMessage, useSessionStore } from "@/features/session/sessionStore";
import { canonicalDocPath } from "@/features/session/toolDisplay";
import { isBusy, type PendingQuestion } from "@/features/session/transcript";
import type { ShellTab } from "@/features/session/workstreamTabs";
import { useWrittenDocsStore } from "@/features/session/writtenDocs";
import { commands } from "@/lib/bindings";
import { unwrap } from "@/lib/ipc";

import { isDraftStatus, parsePlanDoc } from "./planDoc";
import { approvePlan } from "./planDocActions";
import { usePlanPreviewStore } from "./planPreviewStore";

export { isApprovalMessage };

export const STILL_WRITING = "Still writing…";
export const WAITING_FOR_PLAN = "Waiting for the agent to present the plan";
const APPROVAL_TEXT = "Approved. Build it.";

type ApprovalAction =
  | { kind: "thoughts" }
  | { kind: "permission"; requestId: string }
  | { kind: "message" }
  | { kind: "question" };

type ApprovalContext = {
  key: string;
  // A thoughts doc; plans outside the index (~/.claude/plans, inline text) are never written to.
  indexed: boolean;
  status: string | null;
  presentation: PlanPresentation | null;
  busy: boolean;
  pendingPermissionId: string | null;
  pendingQuestion: boolean;
};

type Approval = { action: ApprovalAction | null; blocked: string | null };

const NONE: Approval = { action: null, blocked: null };
const blocked = (reason: string): Approval => ({ action: null, blocked: reason });

// Each harness approves its own way: Claude's ExitPlanMode prompt, OpenCode's plan_exit question,
// a turn for codex's inline plan, and a new implement session for a thoughts plan.
export function approvalFor(c: ApprovalContext): Approval {
  const p = c.presentation;
  if (p?.state === "approved") return NONE;
  if (p?.tool === "ExitPlanMode") {
    const pending = p.state === "pending" && p.requestId !== null && p.requestId === c.pendingPermissionId;
    return pending ? { action: { kind: "permission", requestId: p.requestId! }, blocked: null } : blocked(WAITING_FOR_PLAN);
  }
  if (p?.tool === "plan_exit" && p.state === "pending" && c.pendingQuestion) {
    return { action: { kind: "question" }, blocked: null };
  }
  if (p?.tool === "plan_exit" || isInlinePlanKey(c.key)) {
    return c.busy ? blocked(STILL_WRITING) : { action: { kind: "message" }, blocked: null };
  }
  if (!c.indexed) return blocked(WAITING_FOR_PLAN);
  if (!isDraftStatus(c.status)) return NONE;
  return c.busy ? blocked(STILL_WRITING) : { action: { kind: "thoughts" }, blocked: null };
}

const BUILD_OPTION = /\b(yes|build|approve|implement|switch)\b/i;

function buildAnswers(question: PendingQuestion): Record<string, string> {
  const answers: Record<string, string> = {};
  for (const q of question.questions) {
    const option = q.options.find((o) => BUILD_OPTION.test(o.label)) ?? q.options[0];
    if (option) answers[q.question] = option.label;
  }
  return answers;
}

export async function runApproval(
  action: ApprovalAction,
  ctx: { sessionId: string; workstreamId: string; shell: ShellTab; key: string; pendingQuestion: PendingQuestion | null },
): Promise<void> {
  switch (action.kind) {
    case "thoughts":
      return approvePlan(ctx.workstreamId, ctx.key, ctx.shell);
    case "permission":
      return respondPermission(ctx.sessionId, action.requestId, true, null);
    case "message":
      await sendMessage(ctx.sessionId, APPROVAL_TEXT);
      return;
    case "question":
      if (ctx.pendingQuestion) {
        return answerQuestion(ctx.sessionId, ctx.pendingQuestion.requestId, buildAnswers(ctx.pendingQuestion));
      }
  }
}

// Typing an approval does what the Approve & build button does, instead of sending it.
export async function approveFromChat({ sessionId, workstreamId, shell, text }: OutgoingMessage): Promise<boolean> {
  if (!isApprovalMessage(text)) return false;
  const entry = useSessionStore.getState().sessions[sessionId];
  if (!entry || entry.stage === "implement") return false;
  const written = useWrittenDocsStore.getState().byWorkstream[entry.workstreamId] ?? {};
  const selected = usePlanPreviewStore.getState().bySession[sessionId]?.key;
  const key = selected
    ? isInlinePlanKey(selected)
      ? selected
      : canonicalDocPath(selected, written)
    : planKeyForSession(entry.items, entry.stage, written);
  if (!key) return false;

  let status: string | null = null;
  let indexed = false;
  if (!isInlinePlanKey(key)) {
    // The index can trail a fresh write; send the text rather than swallow it.
    const doc = await unwrap(commands.getDoc(key)).catch(() => null);
    if (!doc) return false;
    status = parsePlanDoc(doc.content, key).status;
    indexed = doc.meta.backend !== "";
  }
  const { action } = approvalFor({
    key,
    indexed,
    status,
    presentation: presentationFor(entry.items, written, key),
    busy: isBusy(entry),
    pendingPermissionId: entry.pendingPermission?.requestId ?? null,
    pendingQuestion: entry.pendingQuestion !== null,
  });
  // A message approval is this very text, so it goes to the agent as typed.
  if (!action || action.kind === "message") return false;
  await runApproval(action, { sessionId, workstreamId, shell, key, pendingQuestion: entry.pendingQuestion });
  return true;
}
