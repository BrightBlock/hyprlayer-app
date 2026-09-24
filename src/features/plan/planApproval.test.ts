import { beforeEach, describe, expect, it, vi } from "vitest";

import { initialSessionView, type SessionView, type TranscriptItem } from "@/features/session/transcript";

const h = vi.hoisted(() => {
  const doc = { content: "", mtimeMs: 1 };
  return {
    doc,
    getDoc: vi.fn(async () => ({ status: "ok", data: { ...doc, meta: {} } })),
    patchDocFrontmatter: vi.fn(async (_id: string, patch: { set: { key: string; value: { value: string } }[] }) => {
      const status = patch.set.find((s) => s.key === "status")!.value.value;
      doc.content = doc.content.replace(/^status: .*$/m, `status: ${status}`);
      return { status: "ok", data: {} };
    }),
    approvePlanAndBuild: vi.fn(async () => {}),
    respondPermission: vi.fn(async () => {}),
    sendMessage: vi.fn(async () => {}),
    answerQuestion: vi.fn(async () => {}),
    sessions: {} as Record<string, SessionView & { stage: string; workstreamId: string }>,
  };
});

vi.mock("@/lib/bindings", () => ({
  commands: { getDoc: h.getDoc, patchDocFrontmatter: h.patchDocFrontmatter },
  events: {},
}));
vi.mock("@/lib/queryClient", () => ({ queryClient: { invalidateQueries: vi.fn() } }));
vi.mock("@/features/session/sessionStore", () => ({
  approvePlanAndBuild: h.approvePlanAndBuild,
  respondPermission: h.respondPermission,
  sendMessage: h.sendMessage,
  answerQuestion: h.answerQuestion,
  useSessionStore: { getState: () => ({ sessions: h.sessions }) },
}));

const { approveFromChat, approvalFor, isApprovalMessage, runApproval, STILL_WRITING, WAITING_FOR_PLAN } =
  await import("./planApproval");
const { approvePlan } = await import("./planDocActions");

const PLAN = "/t/repos/app/shared/plans/2026-09-24-og.md";
const DRAFT = "---\nstatus: draft\n---\n# OG\n\n## Phase 1: A\n- [ ] a\n";
const shell = { id: "sh1" } as never;
const session = (items: TranscriptItem[], stage = "plan", view: Partial<SessionView> = {}) => ({
  ...initialSessionView(),
  items,
  stage,
  workstreamId: "w1",
  ...view,
});
const write: TranscriptItem = {
  kind: "tool",
  id: "w",
  toolUseId: "tu",
  name: "Write",
  input: JSON.stringify({ file_path: PLAN, content: "" }),
  result: { content: "ok", isError: false },
};

beforeEach(() => {
  vi.clearAllMocks();
  h.doc.content = DRAFT;
  h.sessions.s1 = session([write]);
});

describe("isApprovalMessage", () => {
  it.each(["approve", "Approved", "approved.", "Approved. Build it.", "approve and build", "LGTM, build it"])(
    "accepts %s",
    (text) => expect(isApprovalMessage(text)).toBe(true),
  );
  it.each(["approved, but rename phase 2", "not approved", "lgtm", "build it", "I approve of the overview"])(
    "rejects %s",
    (text) => expect(isApprovalMessage(text)).toBe(false),
  );
});

describe("approval from the button and from chat", () => {
  it("both mark the plan active and hand off to an implement session", async () => {
    await approvePlan("w1", PLAN, shell);
    const viaButton = { content: h.doc.content, calls: h.approvePlanAndBuild.mock.calls.slice() };

    h.doc.content = DRAFT;
    h.approvePlanAndBuild.mockClear();
    const handled = await approveFromChat({ sessionId: "s1", workstreamId: "w1", shell, text: "Approved. Build it." });

    expect(handled).toBe(true);
    expect(h.doc.content).toBe(viaButton.content);
    expect(h.doc.content).toContain("status: active");
    expect(h.approvePlanAndBuild.mock.calls).toEqual(viaButton.calls);
  });

  it("lets a non-approval message through to the agent", async () => {
    expect(await approveFromChat({ sessionId: "s1", workstreamId: "w1", shell, text: "rename phase 2" })).toBe(false);
    expect(h.approvePlanAndBuild).not.toHaveBeenCalled();
  });

  it("lets an approval through when the plan is no longer a draft", async () => {
    h.doc.content = DRAFT.replace("draft", "active");
    expect(await approveFromChat({ sessionId: "s1", workstreamId: "w1", shell, text: "approved" })).toBe(false);
    expect(h.approvePlanAndBuild).not.toHaveBeenCalled();
  });

  it("lets an approval through in a session with no plan, or an implement session", async () => {
    h.sessions.s1 = session([]);
    expect(await approveFromChat({ sessionId: "s1", workstreamId: "w1", shell, text: "approved" })).toBe(false);
    h.sessions.s1 = session([write], "implement");
    expect(await approveFromChat({ sessionId: "s1", workstreamId: "w1", shell, text: "approved" })).toBe(false);
  });

  it("lets an approval through while the agent is still writing the plan", async () => {
    h.sessions.s1 = session([write], "plan", { streaming: true });
    expect(await approveFromChat({ sessionId: "s1", workstreamId: "w1", shell, text: "approve" })).toBe(false);
    h.sessions.s1 = session([write], "plan", { state: "awaiting_permission" });
    expect(await approveFromChat({ sessionId: "s1", workstreamId: "w1", shell, text: "approve" })).toBe(false);
    expect(h.approvePlanAndBuild).not.toHaveBeenCalled();
    expect(h.patchDocFrontmatter).not.toHaveBeenCalled();
  });

  it("lets an approval through when the plan can't be read yet", async () => {
    h.getDoc.mockRejectedValueOnce(new Error("not indexed"));
    expect(await approveFromChat({ sessionId: "s1", workstreamId: "w1", shell, text: "approve" })).toBe(false);
    expect(h.approvePlanAndBuild).not.toHaveBeenCalled();
    expect(h.patchDocFrontmatter).not.toHaveBeenCalled();
  });

  it("treats a plan with no status as a draft", async () => {
    h.doc.content = "# OG\n\n## Phase 1: A\n- [ ] a\n";
    expect(await approveFromChat({ sessionId: "s1", workstreamId: "w1", shell, text: "approve" })).toBe(true);
    expect(h.patchDocFrontmatter).toHaveBeenCalledWith(
      PLAN,
      { set: [{ key: "status", value: { kind: "str", value: "active" } }] },
      1,
    );
  });
});

const CLAUDE_PLAN = "/Users/jt/.claude/plans/staged-sleeping-octopus.md";
const presentation = (over: Record<string, unknown>) =>
  ({ toolUseId: "x1", tool: "ExitPlanMode", key: CLAUDE_PLAN, text: "# P", state: "pending", requestId: "r1", ...over }) as never;
const ctx = (over: Record<string, unknown> = {}) => ({
  key: PLAN,
  indexed: true,
  status: "draft" as string | null,
  presentation: null,
  busy: false,
  pendingPermissionId: null as string | null,
  pendingQuestion: false,
  ...over,
});

describe("approvalFor", () => {
  it("builds a draft thoughts plan in a new session, unless the agent is busy or it is not a draft", () => {
    expect(approvalFor(ctx())).toEqual({ action: { kind: "thoughts" }, blocked: null });
    expect(approvalFor(ctx({ busy: true }))).toEqual({ action: null, blocked: STILL_WRITING });
    expect(approvalFor(ctx({ status: "active" }))).toEqual({ action: null, blocked: null });
  });

  it("allows Claude's pending ExitPlanMode, even though the session reads busy while it waits", () => {
    const c = ctx({ key: CLAUDE_PLAN, indexed: false, busy: true, pendingPermissionId: "r1", presentation: presentation({}) });
    expect(approvalFor(c)).toEqual({ action: { kind: "permission", requestId: "r1" }, blocked: null });
  });

  it("waits for Claude to present a plan file it has only written, or is revising", () => {
    expect(approvalFor(ctx({ key: CLAUDE_PLAN, indexed: false, status: null }))).toEqual({ action: null, blocked: WAITING_FOR_PLAN });
    const rejected = ctx({ key: CLAUDE_PLAN, indexed: false, presentation: presentation({ state: "rejected", requestId: null }) });
    expect(approvalFor(rejected)).toEqual({ action: null, blocked: WAITING_FOR_PLAN });
  });

  it("approves an inline codex plan with a message once the turn is over", () => {
    const codex = presentation({ tool: "ProposedPlan", key: "inline", state: "proposed", requestId: null });
    expect(approvalFor(ctx({ key: "inline", indexed: false, presentation: codex }))).toEqual({ action: { kind: "message" }, blocked: null });
    expect(approvalFor(ctx({ key: "inline", indexed: false, presentation: codex, busy: true })).blocked).toBe(STILL_WRITING);
  });

  it("answers OpenCode's plan_exit question when it is pending", () => {
    const exit = presentation({ tool: "plan_exit", key: "/r/.opencode/plans/x.md", text: null, requestId: null });
    expect(approvalFor(ctx({ key: "/r/.opencode/plans/x.md", indexed: false, presentation: exit, busy: true, pendingQuestion: true })).action).toEqual({
      kind: "question",
    });
  });

  it("offers nothing once a presented plan is approved", () => {
    expect(approvalFor(ctx({ key: CLAUDE_PLAN, indexed: false, presentation: presentation({ state: "approved", requestId: null }) }))).toEqual({
      action: null,
      blocked: null,
    });
  });
});

describe("runApproval", () => {
  const base = { sessionId: "s1", workstreamId: "w1", shell, key: CLAUDE_PLAN, pendingQuestion: null };

  it("allows the permission for Claude and never touches the plan file", async () => {
    await runApproval({ kind: "permission", requestId: "r1" }, base);
    expect(h.respondPermission).toHaveBeenCalledWith("s1", "r1", true, null);
    expect(h.patchDocFrontmatter).not.toHaveBeenCalled();
    expect(h.approvePlanAndBuild).not.toHaveBeenCalled();
  });

  it("sends the approval turn for an inline plan", async () => {
    await runApproval({ kind: "message" }, { ...base, key: "inline" });
    expect(h.sendMessage).toHaveBeenCalledWith("s1", "Approved. Build it.");
  });

  it("answers a plan_exit question with its build option", async () => {
    const pendingQuestion = {
      requestId: "q1",
      questions: [
        {
          question: "Switch to the build agent?",
          header: "Build",
          multiSelect: false,
          options: [
            { label: "No, keep planning", description: "" },
            { label: "Yes, start building", description: "" },
          ],
        },
      ],
    };
    await runApproval({ kind: "question" }, { ...base, pendingQuestion } as never);
    expect(h.answerQuestion).toHaveBeenCalledWith("s1", "q1", { "Switch to the build agent?": "Yes, start building" });
  });
});

describe("typed approval per source", () => {
  it("allows Claude's pending ExitPlanMode instead of sending the text", async () => {
    const exit: TranscriptItem = {
      kind: "tool",
      id: "x",
      toolUseId: "x1",
      name: "ExitPlanMode",
      input: JSON.stringify({ plan: "# P", planFilePath: CLAUDE_PLAN }),
      result: null,
    };
    const ask: TranscriptItem = { kind: "permission", id: "p", requestId: "r1", toolName: "ExitPlanMode", input: "{}", decision: null };
    h.sessions.s1 = session([exit, ask], "plan", {
      state: "awaiting_permission",
      pendingPermission: { requestId: "r1", toolName: "ExitPlanMode", input: "{}" },
    });
    h.getDoc.mockResolvedValueOnce({ status: "ok", data: { content: "# P\n", mtimeMs: 1, meta: { backend: "" } } } as never);
    expect(await approveFromChat({ sessionId: "s1", workstreamId: "w1", shell, text: "approved" })).toBe(true);
    expect(h.respondPermission).toHaveBeenCalledWith("s1", "r1", true, null);
    expect(h.patchDocFrontmatter).not.toHaveBeenCalled();
  });

  it("lets a typed approval of an inline codex plan reach the agent, since the text is the approval", async () => {
    const proposed: TranscriptItem = {
      kind: "tool",
      id: "c",
      toolUseId: "c1",
      name: "ProposedPlan",
      input: JSON.stringify({ plan: "# Codex" }),
      result: { content: "Plan proposed", isError: false },
    };
    h.sessions.s1 = session([proposed]);
    expect(await approveFromChat({ sessionId: "s1", workstreamId: "w1", shell, text: "approved" })).toBe(false);
    expect(h.sendMessage).not.toHaveBeenCalled();
  });
});
