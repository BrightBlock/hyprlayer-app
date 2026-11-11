
import type {
  ClientRef,
  ExitOutcome,
  Question,
  QuestionOption,
  SessionEvent,
  SessionState,
} from "@/lib/bindings";
import type { Attachment } from "./attachments";
import { GOAL_PROMPT } from "./goalPrompt";
import { parseToolInput } from "./toolDisplay";

export type { Question, QuestionOption, SessionState };

// "expired": the ask died with its harness process, so no answer can reach it.
export type PermissionDecision = "allow" | "deny" | "expired";

export type PendingQuestion = { requestId: string; questions: Question[] };

// Stored rows written before typed questions still carry a raw JSON string here.
// Removable no earlier than 0.9, once that wire shape is retired.
export function parseQuestions(value: unknown): Question[] {
  let raw = value;
  if (typeof raw === "string") {
    try {
      raw = JSON.parse(raw);
    } catch {
      return [];
    }
  }
  if (!Array.isArray(raw)) return [];
  const out: Question[] = [];
  for (const q of raw) {
    if (q === null || typeof q !== "object") continue;
    const obj = q as Record<string, unknown>;
    const question = typeof obj.question === "string" ? obj.question : "";
    if (!question) continue;
    const options: QuestionOption[] = Array.isArray(obj.options)
      ? obj.options
          .filter((o): o is Record<string, unknown> => o !== null && typeof o === "object")
          .map((o) => ({
            label: typeof o.label === "string" ? o.label : "",
            description: typeof o.description === "string" ? o.description : "",
          }))
          .filter((o) => o.label !== "")
      : [];
    out.push({
      question,
      header: typeof obj.header === "string" ? obj.header : "",
      multiSelect: obj.multiSelect === true,
      options,
    });
  }
  return out;
}

export function buildQuestionAnswers(
  questions: Question[],
  selections: Record<string, string[]>,
  custom: Record<string, string>,
): Record<string, string> {
  const answers: Record<string, string> = {};
  for (const q of questions) {
    const picks = [...(selections[q.question] ?? [])];
    const extra = (custom[q.question] ?? "").trim();
    if (extra) picks.push(extra);
    if (picks.length > 0) answers[q.question] = picks.join(", ");
  }
  return answers;
}

type Parented = { parentId?: string | null };

// The log row an item came from; absent for live-only and optimistic items.
type Sequenced = { seq?: number | null };

const SKILL_BODY_PREFIX = "Base directory for this skill:";

export function skillBodyName(text: string): string | null {
  if (!text.startsWith(SKILL_BODY_PREFIX)) return null;
  const nl = text.indexOf("\n");
  const dir = (nl === -1 ? text : text.slice(0, nl)).slice(SKILL_BODY_PREFIX.length).trim();
  const name = dir.split("/").pop() || dir;
  return name || "skill";
}

// Line-anchored: claude can lead with notice lines, e.g. <artifact-content-authored-by-others/>.
const COMPACT_SUMMARY_START = /^This session is being continued from a previous conversation/m;

export function compactSummaryBody(text: string): string | null {
  const at = text.search(COMPACT_SUMMARY_START);
  return at === -1 ? null : text.slice(at);
}

const THINK_TAG = /<\/?think(?:ing)?>/gi;
// Claude Code sends this as a plain user message after every interrupt.
const INTERRUPT_NOTICE = /^\[Request interrupted by user( for tool use)?\]$/;

const STOP_HOOK_FEEDBACK = "Stop hook feedback:\n";
// Claude's native /goal names its condition in brackets.
const NATIVE_GOAL_FEEDBACK = /^\[[^\]\n]*\]:\s*/;

export type NativeContinuation = { reason: string; native: "hook" | "goal" };

function nativeContinuation(text: string): NativeContinuation | null {
  if (!text.startsWith(STOP_HOOK_FEEDBACK)) return null;
  const body = text.slice(STOP_HOOK_FEEDBACK.length).trim();
  const goal = NATIVE_GOAL_FEEDBACK.exec(body);
  return goal ? { reason: body.slice(goal[0].length).trim(), native: "goal" } : { reason: body, native: "hook" };
}

export function splitInlineThinking(text: string): { reasoning: string | null; answer: string } {
  THINK_TAG.lastIndex = 0;
  if (!THINK_TAG.test(text)) return { reasoning: null, answer: text };
  THINK_TAG.lastIndex = 0;

  const reasoning: string[] = [];
  let answer = "";
  let buf = "";
  let inThink = false;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = THINK_TAG.exec(text)) !== null) {
    buf += text.slice(last, m.index);
    last = m.index + m[0].length;
    const isClose = m[0][1] === "/";
    if (isClose) {
      reasoning.push(inThink ? buf : answer + buf);
      if (!inThink) answer = "";
      buf = "";
      inThink = false;
    } else {
      answer += buf;
      buf = "";
      inThink = true;
    }
  }
  buf += text.slice(last);
  if (inThink) reasoning.push(buf);
  else answer += buf;

  const joined = reasoning.map((r) => r.trim()).filter(Boolean).join("\n\n");
  return joined ? { reasoning: joined, answer: answer.trim() } : { reasoning: null, answer: text };
}

export type TranscriptItem =
  | ({
      kind: "user";
      id: string;
      text: string;
      stage: string | null;
      attachments: Attachment[];
      createdAt: number | null;
      promptName: string | null;
      clientMessageId?: string | null;
      author?: ClientRef | null;
    } & Sequenced)
  | ({ kind: "assistant"; id: string; text: string; streaming: boolean; authRequired?: boolean } & Parented & Sequenced)
  | ({ kind: "thinking"; id: string; text: string } & Parented & Sequenced)
  | ({ kind: "skill"; id: string; name: string; body: string } & Parented & Sequenced)
  | ({
      kind: "tool";
      id: string;
      toolUseId: string;
      name: string;
      input: string;
      result: { content: string; isError: boolean } | null;
      skillBody?: string;
    } & Parented & Sequenced)
  | ({
      kind: "permission";
      id: string;
      requestId: string;
      toolName: string;
      input: string;
      toolUseId?: string | null;
      decision: PermissionDecision | null;
    } & Sequenced)
  | ({ kind: "error"; id: string; detail: string; raw: string } & Sequenced)
  | ({
      kind: "compact";
      id: string;
      trigger: string;
      preTokens: number | null;
      summary?: string;
    } & Sequenced)
  | ({
      kind: "queuedCommand";
      id: string;
      commandUuid: string;
      label: string;
      detail: string | null;
    } & Sequenced)
  | ({
      kind: "taskUpdate";
      id: string;
      taskId: string;
      toolUseId: string | null;
      status: string;
    } & Sequenced)
  | ({ kind: "nativeContinuation"; id: string } & NativeContinuation & Sequenced)
  | ({
      kind: "relaunched";
      id: string;
      quiet?: boolean;
    } & Sequenced)
  | ({
      kind: "rewound";
      id: string;
    } & Sequenced)
  | ({
      kind: "exit";
      id: string;
      outcome: ExitOutcome;
      code: number | null;
      stderrTail: string;
    } & Sequenced);

export type SessionView = {
  items: TranscriptItem[];
  counter: number;
  state: SessionState;
  streaming: boolean;
  costUsd: number | null;
  numTurns: number | null;
  contextTokens: number | null;
  contextWindow: number | null;
  model: string | null;
  claudeSessionId: string | null;
  mcpServers: string[];
  pendingPermission: { requestId: string; toolName: string; input: string } | null;
  pendingQuestion: PendingQuestion | null;
};

export function initialSessionView(state: SessionState = "idle"): SessionView {
  return {
    items: [],
    counter: 0,
    state,
    streaming: false,
    costUsd: null,
    numTurns: null,
    contextTokens: null,
    contextWindow: null,
    model: null,
    claudeSessionId: null,
    mcpServers: [],
    pendingPermission: null,
    pendingQuestion: null,
  };
}

const TERMINAL: ReadonlySet<SessionState> = new Set<SessionState>([
  "completed",
  "interrupted",
  "crashed",
  "killed",
  "spawn_failed",
]);

export function isTerminal(state: SessionState): boolean {
  return TERMINAL.has(state);
}

export function hasPendingPermission(view: SessionView): boolean {
  return view.items.some((it) => it.kind === "permission" && it.decision === null);
}

export function isBusy(view: SessionView): boolean {
  if (isTerminal(view.state)) return false;
  return (
    view.state === "starting" ||
    view.state === "awaiting_permission" ||
    view.state === "awaiting_question" ||
    view.streaming ||
    hasPendingPermission(view) ||
    view.pendingQuestion !== null
  );
}

export function isWorking(view: SessionView): boolean {
  if (isTerminal(view.state)) return false;
  if (view.state === "awaiting_permission" || view.state === "awaiting_question") return false;
  if (hasPendingPermission(view) || view.pendingQuestion !== null) return false;
  return view.state === "starting" || view.streaming;
}

export function isAuthExit(stderrTail: string): boolean {
  return /\/login\b|not logged in|unauthor/i.test(stderrTail);
}

/**
 * The auth prompt nothing has answered yet: no message since and no turn running.
 * A relaunch answers it only when it had nothing to resend (a goal's prompt) or it was an auth exit.
 */
export function openAuthPrompt(view: SessionView): string | null {
  if (isBusy(view)) return null;
  let relaunched = false;
  for (let i = view.items.length - 1; i >= 0; i -= 1) {
    const it = view.items[i];
    if (it.kind === "user") return null;
    if (it.kind === "relaunched") relaunched = true;
    if (it.kind === "assistant" && it.authRequired) {
      return relaunched && messageStoppedByAuth(view.items)?.promptName === GOAL_PROMPT ? null : it.id;
    }
    const authExit =
      it.kind === "exit" && (it.outcome === "crashed" || it.outcome === "spawn_failed") && isAuthExit(it.stderrTail);
    if (authExit) return isTerminal(view.state) ? it.id : null;
  }
  return null;
}

/** The message the open auth prompt stopped, to send again once signed in. */
export function messageStoppedByAuth(
  items: TranscriptItem[],
): Extract<TranscriptItem, { kind: "user" }> | null {
  let prompted = false;
  for (let i = items.length - 1; i >= 0; i -= 1) {
    const it = items[i];
    if (it.kind === "assistant" && it.authRequired) prompted = true;
    else if (it.kind === "user") return prompted ? it : null;
  }
  return null;
}

export function parkedState(view: SessionView): SessionState {
  if (isTerminal(view.state)) return view.state;
  if (view.pendingQuestion !== null) return "awaiting_question";
  if (hasPendingPermission(view)) return "awaiting_permission";
  return "running";
}

export function restoredView(view: SessionView, live: boolean): SessionView {
  if (isTerminal(view.state)) return { ...view, streaming: false };
  const settled = live ? view : expirePendingPrompts(view);
  if (settled.pendingQuestion !== null || hasPendingPermission(settled)) {
    return { ...settled, state: parkedState(settled), streaming: false };
  }
  return {
    ...settled,
    state: "idle",
    streaming: false,
    pendingPermission: null,
    pendingQuestion: null,
  };
}

// Its card's decision was local only; a restore would offer it again, and turnDone would park on it.
function settleSubagentAsk(view: SessionView, items: TranscriptItem[], toolUseId: string): SessionView {
  const i = items.findIndex((it) => it.kind === "permission" && it.decision === null && it.toolUseId === toolUseId);
  if (i === -1) return { ...view, items };
  const ask = items[i] as Extract<TranscriptItem, { kind: "permission" }>;
  const settled = items.slice();
  settled[i] = { ...ask, decision: "allow" };
  const pendingPermission = view.pendingPermission?.requestId === ask.requestId ? null : view.pendingPermission;
  const next = { ...view, items: settled, pendingPermission };
  return { ...next, state: parkedState(next) };
}

function settleAnsweredPrompt(
  view: SessionView,
  items: TranscriptItem[],
  toolName: string,
  toolUseId: string,
): SessionView {
  let nextItems = items;
  let pendingPermission = view.pendingPermission;
  let pendingQuestion = view.pendingQuestion;

  if (toolName === "AskUserQuestion") {
    pendingQuestion = null;
  } else {
    for (let i = nextItems.length - 1; i >= 0; i -= 1) {
      const it = nextItems[i];
      if (it.kind !== "permission" || it.decision !== null) continue;
      // Asks from older servers carry no tool use id; the name is all they have.
      if (it.toolUseId ? it.toolUseId !== toolUseId : it.toolName !== toolName) continue;
      nextItems = nextItems.slice();
      nextItems[i] = { ...it, decision: "allow" };
      if (pendingPermission?.requestId === it.requestId) pendingPermission = null;
      break;
    }
  }

  const next: SessionView = {
    ...view,
    items: nextItems,
    pendingPermission,
    pendingQuestion,
    streaming: true,
  };
  return { ...next, state: parkedState(next) };
}


function openAssistantIndex(items: TranscriptItem[]): number {
  for (let i = items.length - 1; i >= 0; i -= 1) {
    const it = items[i];
    if (it.kind === "assistant" && it.streaming && !it.parentId) return i;
  }
  return -1;
}

function streamedAssistantIndex(items: TranscriptItem[], text: string): number {
  if (!text.length) return -1;
  for (let i = items.length - 1; i >= 0; i -= 1) {
    const it = items[i];
    if (it.kind === "tool" || it.kind === "thinking") continue;
    if (it.kind === "assistant" && !it.parentId && it.text.length > 0 && text.startsWith(it.text)) return i;
    return -1;
  }
  return -1;
}

const SKILL_LAUNCH_RESULT = "Launching skill:";

function firstSkillToolAwaitingBody(items: TranscriptItem[], parentId: string | null): number {
  for (let i = 0; i < items.length; i += 1) {
    const it = items[i];
    if (
      it.kind === "tool" &&
      it.name === "Skill" &&
      it.result !== null &&
      !it.result.isError &&
      it.result.content.startsWith(SKILL_LAUNCH_RESULT) &&
      it.skillBody === undefined &&
      (it.parentId ?? null) === parentId
    ) {
      return i;
    }
  }
  return -1;
}

// Only the latest divider: an older one that never got its summary must not take this one.
function latestCompactAwaitingSummary(items: TranscriptItem[]): number {
  for (let i = items.length - 1; i >= 0; i -= 1) {
    const it = items[i];
    if (it.kind === "compact") return it.summary === undefined ? i : -1;
  }
  return -1;
}

function queuedCommandSeam(items: TranscriptItem[]): { label: string; detail: string | null } {
  const generic = { label: "Queued command", detail: null };
  for (let i = items.length - 1; i >= 0; i -= 1) {
    const it = items[i];
    if (it.kind !== "tool") continue;
    if (it.name !== "ScheduleWakeup") return generic;
    const input = parseToolInput(it.input);
    const reason = input?.reason ?? input?.prompt;
    return {
      label: "Scheduled wakeup",
      detail: typeof reason === "string" && reason.trim() ? reason.trim() : null,
    };
  }
  return generic;
}

export function closeOpenAssistant(items: TranscriptItem[]): TranscriptItem[] {
  const idx = openAssistantIndex(items);
  if (idx === -1) return items;
  const block = items[idx] as Extract<TranscriptItem, { kind: "assistant" }>;
  const copy = items.slice();
  copy[idx] = { ...block, streaming: false };
  return copy;
}

// Every caller passes a freshly built item, so stamping in place saves a copy per item.
function withSeq(item: TranscriptItem, seq: number | null): TranscriptItem {
  if (seq !== null) item.seq = seq;
  return item;
}

function appended(
  view: SessionView,
  make: (id: string) => TranscriptItem,
  closeAssistant = false,
  seq: number | null = null,
): Pick<SessionView, "items" | "counter"> {
  const base = closeAssistant ? closeOpenAssistant(view.items) : view.items;
  return {
    items: [...base, withSeq(make(`i${view.counter}`), seq)],
    counter: view.counter + 1,
  };
}


export function reduceEvent(view: SessionView, event: SessionEvent, seq: number | null = null): SessionView {
  const inner = reduceEventInner(view, event, seq);
  // A background subagent's events keep arriving after the launching turn's
  // own turnDone; letting one raise `streaming` pins the session at "working"
  // from the moment its work stops until the next turn.
  const next =
    "parentId" in event && event.parentId && inner.streaming && !view.streaming
      ? { ...inner, streaming: false }
      : inner;
  const flagged = event.kind === "assistantText" && event.authRequired && !event.parentId ? flagAuthText(next) : next;
  if (flagged.state === "starting" && flagged.streaming) {
    return { ...flagged, state: "running" };
  }
  return flagged;
}

// The text may have merged into a streamed block, so the flag goes on whichever item holds it now.
function flagAuthText(view: SessionView): SessionView {
  for (let i = view.items.length - 1; i >= 0; i -= 1) {
    const it = view.items[i];
    if (it.kind !== "assistant" || it.parentId) continue;
    const items = view.items.slice();
    items[i] = { ...it, authRequired: true };
    return { ...view, items };
  }
  return view;
}

function reduceEventInner(view: SessionView, event: SessionEvent, seq: number | null): SessionView {
  switch (event.kind) {
    case "init": {
      // A new id is a new conversation (e.g. /clear): its fill is unknown until reported.
      const fresh = !!view.claudeSessionId && !!event.claudeSessionId && event.claudeSessionId !== view.claudeSessionId;
      return {
        ...view,
        state: "running",
        claudeSessionId: event.claudeSessionId || view.claudeSessionId,
        model: event.model ?? view.model,
        mcpServers: event.mcpServers ?? view.mcpServers,
        contextWindow: event.model && event.model !== view.model ? null : view.contextWindow,
        contextTokens: fresh ? null : view.contextTokens,
      };
    }

    case "userMessage": {
      // The sender already shows this turn optimistically; only its own echo is dropped.
      const id = event.clientMessageId;
      if (id && view.items.some((it) => it.kind === "user" && it.clientMessageId === id)) return view;
      return {
        ...view,
        ...appended(
          view,
          (itemId) => ({
            kind: "user",
            id: itemId,
            text: event.text,
            stage: event.stage,
            attachments: [],
            createdAt: null,
            promptName: event.promptName ?? null,
            clientMessageId: id ?? null,
            author: event.author ?? null,
          }),
          true,
          seq,
        ),
        streaming: true,
      };
    }

    case "partialText": {
      if (event.parentId || !event.text) return view;
      const idx = openAssistantIndex(view.items);
      if (idx !== -1) {
        const block = view.items[idx] as Extract<TranscriptItem, { kind: "assistant" }>;
        const items = view.items.slice();
        items[idx] = { ...block, text: block.text + event.text };
        return { ...view, items, streaming: true };
      }
      return {
        ...view,
        ...appended(view, (id) => ({ kind: "assistant", id, text: event.text, streaming: true }), false, seq),
        streaming: true,
      };
    }

    case "assistantText": {
      if (INTERRUPT_NOTICE.test(event.text)) return view;
      // Ahead of the skill fold, which would take a hook line as a prefix-less skill body.
      const continuation = event.injected ? nativeContinuation(event.text) : null;
      if (continuation) {
        return { ...view, ...appended(view, (id) => ({ kind: "nativeContinuation", id, ...continuation }), true, seq) };
      }
      // Ahead of the skill fold: the summary arrives injected too.
      const summary = event.parentId ? null : compactSummaryBody(event.text);
      if (summary !== null) {
        const open = openAssistantIndex(view.items);
        const base = open === -1 ? view.items : view.items.filter((_, i) => i !== open);
        const compactIdx = latestCompactAwaitingSummary(base);
        if (compactIdx !== -1) {
          const compact = base[compactIdx] as Extract<TranscriptItem, { kind: "compact" }>;
          const items = base.slice();
          items[compactIdx] = { ...compact, summary };
          return { ...view, items, streaming: true };
        }
      }
      const skill = skillBodyName(event.text);
      if (skill || event.injected) {
        const toolIdx = firstSkillToolAwaitingBody(view.items, event.parentId ?? null);
        if (toolIdx !== -1) {
          const tool = view.items[toolIdx] as Extract<TranscriptItem, { kind: "tool" }>;
          const items = view.items.slice();
          items[toolIdx] = { ...tool, skillBody: event.text };
          return { ...view, items, streaming: true };
        }
      }
      if (skill) {
        const makeSkill = (id: string): TranscriptItem => ({
          kind: "skill",
          id,
          name: skill,
          body: event.text,
          parentId: event.parentId,
        });
        if (!event.parentId) {
          const open = openAssistantIndex(view.items);
          if (open !== -1) {
            const items = view.items.slice();
            items[open] = makeSkill(view.items[open].id);
            return { ...view, items, streaming: true };
          }
        }
        return {
          ...view,
          ...appended(view, makeSkill, !event.parentId, seq),
          streaming: true,
        };
      }
      // Any other injected note (image sizes, resume nudges, hook feedback) is for the model.
      if (event.injected) return view;
      if (event.parentId) {
        if (!event.text.length) return view;
        return {
          ...view,
          ...appended(view, (id) => ({
            kind: "assistant",
            id,
            text: event.text,
            streaming: false,
            parentId: event.parentId,
          }), false, seq),
          streaming: true,
        };
      }
      const { reasoning, answer } = splitInlineThinking(event.text);
      if (reasoning) {
        const open = openAssistantIndex(view.items);
        const at = open !== -1 ? open : streamedAssistantIndex(view.items, event.text);
        const built: TranscriptItem[] = [
          withSeq({ kind: "thinking", id: `i${view.counter}`, text: reasoning, parentId: null }, seq),
        ];
        if (answer.length)
          built.push(withSeq({ kind: "assistant", id: `i${view.counter + 1}`, text: answer, streaming: false }, seq));
        const items =
          at === -1
            ? [...view.items, ...built]
            : [...view.items.slice(0, at), ...built, ...view.items.slice(at + 1)];
        return { ...view, items, counter: view.counter + built.length, streaming: true };
      }

      const idx = openAssistantIndex(view.items);
      if (idx !== -1) {
        const block = view.items[idx] as Extract<TranscriptItem, { kind: "assistant" }>;
        const items = view.items.slice();
        items[idx] = {
          ...block,
          text: event.text.length ? event.text : block.text,
          streaming: false,
        };
        return { ...view, items, streaming: true };
      }
      if (!event.text.length) return view;
      const streamed = streamedAssistantIndex(view.items, event.text);
      if (streamed !== -1) {
        const block = view.items[streamed] as Extract<TranscriptItem, { kind: "assistant" }>;
        const items = view.items.slice();
        items[streamed] = { ...block, text: event.text, streaming: false };
        return { ...view, items, streaming: true };
      }
      return {
        ...view,
        ...appended(view, (id) => ({ kind: "assistant", id, text: event.text, streaming: false }), false, seq),
        streaming: true,
      };
    }

    case "thinking": {
      if (!event.text.length) return view;
      return {
        ...view,
        ...appended(
          view,
          (id) => ({ kind: "thinking", id, text: event.text, parentId: event.parentId }),
          !event.parentId,
          seq,
        ),
        streaming: true,
      };
    }

    case "toolUse": {
      return {
        ...view,
        ...appended(
          view,
          (id) => ({
            kind: "tool",
            id,
            toolUseId: event.id,
            name: event.name,
            input: event.input,
            result: null,
            parentId: event.parentId,
          }),
          !event.parentId,
          seq,
        ),
        streaming: true,
      };
    }

    case "toolResult": {
      for (let i = view.items.length - 1; i >= 0; i -= 1) {
        const item = view.items[i];
        if (item.kind === "tool" && item.toolUseId === event.toolUseId && item.result === null) {
          const items = view.items.slice();
          items[i] = { ...item, result: { content: event.content, isError: event.isError } };
          return event.parentId
            ? settleSubagentAsk(view, items, event.toolUseId)
            : settleAnsweredPrompt(view, items, item.name, event.toolUseId);
        }
      }
      return {
        ...view,
        ...appended(view, (id) => ({
          kind: "tool",
          id,
          toolUseId: event.toolUseId,
          name: "result",
          input: "",
          result: { content: event.content, isError: event.isError },
          parentId: event.parentId,
        }), false, seq),
        streaming: true,
      };
    }

    case "permissionAsk": {
      return {
        ...view,
        ...appended(
          view,
          (id) => ({
            kind: "permission",
            id,
            requestId: event.requestId,
            toolName: event.toolName,
            input: event.input,
            toolUseId: event.toolUseId ?? null,
            decision: null,
          }),
          true,
          seq,
        ),
        state: "awaiting_permission",
        streaming: view.streaming || !isSubagentTool(view.items, event.toolUseId),
        pendingPermission: {
          requestId: event.requestId,
          toolName: event.toolName,
          input: event.input,
        },
      };
    }

    case "questionAsk": {
      return {
        ...view,
        items: closeOpenAssistant(view.items),
        state: "awaiting_question",
        streaming: true,
        pendingQuestion: {
          requestId: event.requestId,
          questions: parseQuestions(event.questions),
        },
      };
    }

    case "usage":
      return { ...view, contextTokens: event.contextTokens };

    case "contextWindow":
      return { ...view, contextWindow: event.tokens };

    case "turnDone": {
      return {
        ...view,
        items: closeOpenAssistant(view.items),
        state: parkedState(view),
        streaming: false,
        costUsd: event.costUsd ?? view.costUsd,
        numTurns: event.numTurns ?? view.numTurns,
        contextTokens: event.contextTokens ?? view.contextTokens,
      };
    }

    case "exited": {
      const closed = expirePendingPrompts({ ...view, items: closeOpenAssistant(view.items) });
      if (event.outcome === "detached") {
        return { ...closed, streaming: false, state: "idle" };
      }
      return {
        ...closed,
        ...appended(
          closed,
          (id) => ({
            kind: "exit",
            id,
            outcome: event.outcome,
            code: event.code,
            stderrTail: event.stderrTail,
          }),
          true,
          seq,
        ),
        state: event.outcome,
        streaming: false,
        pendingPermission: null,
        pendingQuestion: null,
      };
    }

    case "protocolError":
      return {
        ...view,
        ...appended(view, (id) => ({ kind: "error", id, detail: event.detail, raw: event.raw }), false, seq),
      };

    case "commandLifecycle":
      if (event.state !== "started") return view;
      return {
        ...view,
        ...appended(
          view,
          (id) => ({
            kind: "queuedCommand",
            id,
            commandUuid: event.commandUuid,
            ...queuedCommandSeam(view.items),
          }),
          true,
          seq,
        ),
      };

    case "compactBoundary":
      // The pre-compaction fill is stale; a usage after the boundary restores it.
      return {
        ...view,
        contextTokens: null,
        ...appended(
          view,
          (id) => ({
            kind: "compact",
            id,
            trigger: event.trigger,
            preTokens: event.preTokens,
          }),
          true,
          seq,
        ),
      };

    case "relaunched":
      return {
        ...view,
        ...appended(view, (id) => ({ kind: "relaunched", id, quiet: event.quiet }), false, seq),
      };

    case "rewound":
      return {
        ...view,
        ...appended(view, (id) => ({ kind: "rewound", id }), false, seq),
      };

    case "backgroundTaskUpdate":
      return {
        ...view,
        ...appended(view, (id) => ({
          kind: "taskUpdate",
          id,
          taskId: event.taskId,
          toolUseId: event.toolUseId,
          status: event.status,
        }), false, seq),
      };

    default:
      return view;
  }
}


export function addUserMessage(
  view: SessionView,
  text: string,
  stage: string | null,
  attachments: Attachment[] = [],
  createdAt: number | null = null,
  promptName: string | null = null,
  clientMessageId: string | null = null,
): SessionView {
  return {
    ...view,
    ...appended(
      view,
      (id) => ({ kind: "user", id, text, stage, attachments, createdAt, promptName, clientMessageId }),
      true,
    ),
    streaming: true,
  };
}

// A background subagent asks after its launching turn ended; answering it doesn't reopen that turn.
function isSubagentTool(items: TranscriptItem[], toolUseId: string | null | undefined): boolean {
  return !!toolUseId && items.some((it) => it.kind === "tool" && it.toolUseId === toolUseId && !!it.parentId);
}

export function resolvePermission(
  view: SessionView,
  requestId: string,
  decision: PermissionDecision,
): SessionView {
  const asked = view.items.find((it) => it.kind === "permission" && it.requestId === requestId);
  const subagent = asked?.kind === "permission" && isSubagentTool(view.items, asked.toolUseId);
  const items = view.items.map((item) =>
    item.kind === "permission" && item.requestId === requestId && item.decision === null
      ? { ...item, decision }
      : item,
  );
  const pendingPermission =
    view.pendingPermission?.requestId === requestId ? null : view.pendingPermission;
  const next = { ...view, items, pendingPermission };
  return { ...next, state: parkedState(next), streaming: view.streaming || !subagent };
}

export function resolveQuestion(view: SessionView, requestId: string): SessionView {
  if (view.pendingQuestion?.requestId !== requestId) return view;
  const next = { ...view, pendingQuestion: null };
  return { ...next, state: parkedState(next), streaming: true };
}

export function expirePendingPrompts(view: SessionView): SessionView {
  const items: TranscriptItem[] = view.items.map((it) =>
    it.kind === "permission" && it.decision === null ? { ...it, decision: "expired" as const } : it,
  );
  let counter = view.counter;
  // A question has no transcript card of its own, so it leaves one behind.
  if (view.pendingQuestion) {
    items.push({
      kind: "permission",
      id: `i${counter}`,
      requestId: view.pendingQuestion.requestId,
      toolName: "AskUserQuestion",
      input: JSON.stringify({ questions: view.pendingQuestion.questions }),
      decision: "expired",
    });
    counter += 1;
  }
  return { ...view, items, counter, pendingPermission: null, pendingQuestion: null };
}

export function cancelPendingPrompts(view: SessionView): SessionView {
  const items = view.items.map((it) =>
    it.kind === "permission" && it.decision === null ? { ...it, decision: "deny" as const } : it,
  );
  return { ...view, items, pendingPermission: null, pendingQuestion: null };
}


export type StatusTone = "ok" | "warn" | "bad" | "dim" | "accent";

export function describeState(
  state: SessionState,
  streaming: boolean,
): { label: string; tone: StatusTone } {
  switch (state) {
    case "idle":
      return { label: "idle", tone: "dim" };
    case "starting":
      return { label: "starting", tone: "accent" };
    case "running":
      return streaming
        ? { label: "working", tone: "accent" }
        : { label: "ready", tone: "ok" };
    case "awaiting_permission":
      return { label: "needs approval", tone: "warn" };
    case "awaiting_question":
      return { label: "needs answer", tone: "warn" };
    case "completed":
      return { label: "done", tone: "ok" };
    case "interrupted":
      return { label: "stopped", tone: "dim" };
    case "crashed":
      return { label: "crashed", tone: "bad" };
    case "killed":
      return { label: "ended", tone: "dim" };
    case "spawn_failed":
      return { label: "failed", tone: "bad" };
    default:
      return { label: state, tone: "dim" };
  }
}

const EXIT_LABEL: Record<ExitOutcome, string> = {
  completed: "Session completed",
  interrupted: "Turn interrupted",
  crashed: "Session crashed",
  killed: "Session ended",
  detached: "Session ended",
  spawn_failed: "Failed to start session",
};

export function exitLabel(outcome: ExitOutcome): string {
  return EXIT_LABEL[outcome] ?? "Session ended";
}

export function viewToText(view: SessionView): string {
  const lines: string[] = [];
  for (const item of view.items) {
    switch (item.kind) {
      case "user": {
        if (item.promptName === GOAL_PROMPT) {
          lines.push("--- Goal iteration ---", "");
          break;
        }
        const heading = `## You${item.stage && item.stage !== "general" ? ` (${item.stage})` : ""}`;
        lines.push(heading);
        if (item.promptName) lines.push(`*Prompt: ${item.promptName}*`);
        lines.push(item.text);
        for (const att of item.attachments ?? []) lines.push(`📎 ${att.name}`);
        lines.push("");
        break;
      }
      case "assistant":
        lines.push("## Assistant", item.text, "");
        break;
      case "thinking":
        lines.push("> [thinking]", ...item.text.split("\n").map((l) => `> ${l}`), "");
        break;
      case "skill":
        lines.push(`- skill: ${item.name}`, "");
        break;
      case "tool": {
        const header = `- tool: ${item.name}`;
        lines.push(item.input ? `${header} ${item.input}` : header);
        if (item.result) {
          lines.push(`  ${item.result.isError ? "error" : "result"}: ${item.result.content}`);
        }
        lines.push("");
        break;
      }
      case "permission":
        lines.push(
          `- permission: ${item.toolName} → ${item.decision ?? "pending"}`,
          "",
        );
        break;
      case "error":
        lines.push(`- protocol error: ${item.detail}`, "");
        break;
      case "compact":
        lines.push("--- Conversation compacted ---", "");
        break;
      case "nativeContinuation":
        lines.push(`- stop hook: ${item.reason}`, "");
        break;
      case "queuedCommand":
        lines.push(`--- ${item.label} ---`, "");
        break;
      case "exit":
        lines.push(`--- ${exitLabel(item.outcome)} ---`, "");
        break;
    }
  }
  return lines.join("\n").trim();
}
