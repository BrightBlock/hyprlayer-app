/**
 * The pure event→transcript reducer for an agent session. Given the ordered
 * stream of `SessionEvent`s the driver emits, it builds the renderable
 * transcript and the derived session state. No React, no IPC, no clocks — so
 * it's fully deterministic and unit-testable, and it runs identically whether
 * events arrive live over a Channel or are replayed from SQLite on re-attach.
 *
 * Coalescing rules (verified against the claude 2.1.x stream-json protocol):
 *  - `partialText` deltas accumulate into the open streaming assistant block;
 *  - the matching `assistantText` (the completed block) REPLACES that text and
 *    closes the block — it is authoritative, never appended (would double);
 *  - `toolUse` opens a tool block; the later `toolResult` (echoed on a `user`
 *    turn) attaches to it by `toolUseId`;
 *  - `permissionAsk` parks the session in `awaiting_permission`; the answer
 *    (a frontend action, no backend event) is applied via `resolvePermission`.
 */

import type { ExitOutcome, SessionEvent } from "../../lib/bindings";
import type { Attachment } from "./attachments";

export type SessionState =
  | "idle"
  | "starting"
  | "running"
  | "awaiting_permission"
  | "awaiting_question"
  | ExitOutcome;

export type PermissionDecision = "allow" | "deny";

/** One selectable answer to an `AskUserQuestion` question. */
export type QuestionOption = { label: string; description: string };

/** A single question from an `AskUserQuestion` call. */
export type Question = {
  question: string;
  header: string;
  multiSelect: boolean;
  options: QuestionOption[];
};

/** The unanswered `AskUserQuestion` prompt the session is parked on — rendered
 *  interactively above the composer (not as a transcript card). */
export type PendingQuestion = { requestId: string; questions: Question[] };

/**
 * Parse the `questions` JSON string from a `questionAsk` event into typed
 * questions, dropping anything malformed so a bad payload can't crash the UI.
 */
export function parseQuestions(json: string): Question[] {
  let raw: unknown;
  try {
    raw = JSON.parse(json);
  } catch {
    return [];
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

/**
 * Build the `answers` map sent back for an `AskUserQuestion` from the user's
 * option picks plus any "Other" free-text. For each question the selected option
 * labels and a trimmed custom answer are merged (custom appended last) and
 * comma-joined — the shape the driver feeds to claude. Questions with neither a
 * pick nor custom text are omitted; an all-empty result is the "dismissed"
 * signal (the agent then re-asks in prose).
 */
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

/** Set on items produced inside a subagent (the `Task` tool-use id); the UI
 *  nests them under that Task instead of inlining them in the main thread. */
type Parented = { parentId?: string | null };

/** Prefix Claude Code prepends to a launched skill's body: the complete
 *  assistant block reads `Base directory for this skill: <dir>\n\n<SKILL.md>`. */
const SKILL_BODY_PREFIX = "Base directory for this skill:";

/**
 * If a completed assistant block is actually a launched skill's injected body
 * (it starts with [`SKILL_BODY_PREFIX`]), return the skill's name — the last
 * segment of the base-directory path on the first line — else null. Matched on
 * the *complete* block only: a streamed `partialText` chunk can start with the
 * prefix by coincidence, but a whole assistant turn starting with it is the
 * skill injection, not prose.
 */
export function skillBodyName(text: string): string | null {
  if (!text.startsWith(SKILL_BODY_PREFIX)) return null;
  const nl = text.indexOf("\n");
  const dir = (nl === -1 ? text : text.slice(0, nl)).slice(SKILL_BODY_PREFIX.length).trim();
  const name = dir.split("/").pop() || dir;
  return name || "skill";
}

/** The system-injected preamble that opens claude's post-compact summary —
 *  delivered as one (often 10–20KB) assistant block right after the compact
 *  boundary. Matched to fold that block into the divider instead of dumping it. */
const COMPACT_SUMMARY_PREFIX = "This session is being continued from a previous conversation";

/**
 * True when a completed assistant block is claude's post-compact summary (it
 * opens with [`COMPACT_SUMMARY_PREFIX`]). Matched on the *complete* block, like
 * [`skillBodyName`]: a streamed `partialText` chunk could start this way by
 * coincidence, but a whole assistant turn opening with it is the injected summary.
 */
export function isCompactSummary(text: string): boolean {
  return text.startsWith(COMPACT_SUMMARY_PREFIX);
}

export type TranscriptItem =
  | {
      kind: "user";
      id: string;
      text: string;
      stage: string | null;
      /** Files/screenshots attached to this turn — rendered as pin cards, while
       *  the path the agent actually received stays out of the bubble. */
      attachments: Attachment[];
      /** Epoch ms the user sent this turn — stamped optimistically by
       *  `addUserMessage`, or backfilled from the persisted event's `createdAt`
       *  on replay. `null` only for legacy rows logged before this field
       *  existed; the UI falls back to the session's start time in that case. */
      createdAt: number | null;
    }
  | ({ kind: "assistant"; id: string; text: string; streaming: boolean } & Parented)
  | ({ kind: "thinking"; id: string; text: string } & Parented)
  /** A launched skill's injected SKILL.md body — Claude Code delivers it as an
   *  assistant block; we keep it as its own collapsed chip instead of dumping the
   *  whole instruction text inline in the thread. */
  | ({ kind: "skill"; id: string; name: string; body: string } & Parented)
  | ({
      kind: "tool";
      id: string;
      toolUseId: string;
      name: string;
      input: string;
      result: { content: string; isError: boolean } | null;
      /** For a `Skill` call: the launched skill's injected SKILL.md body, folded
       *  into this row (expand to read it) instead of dumped as its own block. */
      skillBody?: string;
    } & Parented)
  | {
      kind: "permission";
      id: string;
      requestId: string;
      toolName: string;
      input: string;
      decision: PermissionDecision | null;
    }
  | { kind: "error"; id: string; detail: string; raw: string }
  /** A compact-boundary marker — claude auto- or manually compacted the
   *  conversation. Rendered as a thin divider so a silent stretch during compact
   *  is legible. `trigger` is "auto" or "manual"; `preTokens` is the context
   *  fill just before the compact, when reported. `summary` is claude's
   *  post-compact summary — the large block injected right after the boundary,
   *  folded in here so it expands from the divider instead of dumping inline. */
  | {
      kind: "compact";
      id: string;
      trigger: string;
      preTokens: number | null;
      summary?: string;
    }
  /** A background shell task (`Bash{run_in_background}`) changed state — claude's
   *  asynchronous "you will be notified when it completes" signal, surfaced so the
   *  live background-tasks indicator drops a finished task even when the agent
   *  never polled it. A non-rendering control marker (it produces no UI in the
   *  transcript, like a `compact` divider with no body); consumed only by
   *  `backgroundTasksFromItems`. `status` is "completed"/"failed" at terminal —
   *  anything but "running" ends the task. `toolUseId` is the spawning `Bash`
   *  call when known (the direct key), else null and matched via `taskId`. */
  | {
      kind: "taskUpdate";
      id: string;
      taskId: string;
      toolUseId: string | null;
      status: string;
    }
  /** Non-rendering marker for the backend's `relaunched` event (a new process
   *  took over the session). `backgroundTasksFromItems` drops the dead prior
   *  generation's background tasks at it. */
  | {
      kind: "relaunched";
      id: string;
    }
  | {
      kind: "exit";
      id: string;
      outcome: ExitOutcome;
      code: number | null;
      stderrTail: string;
    };

export type SessionView = {
  items: TranscriptItem[];
  /** Monotonic counter for stable, deterministic React keys. */
  counter: number;
  state: SessionState;
  /** A turn is in flight (between a user turn and the next `turnDone`). */
  streaming: boolean;
  costUsd: number | null;
  numTurns: number | null;
  /** Input tokens in context as of the last finished turn (the meter's fill). */
  contextTokens: number | null;
  model: string | null;
  claudeSessionId: string | null;
  /** The unanswered permission ask, if the session is parked on one. */
  pendingPermission: { requestId: string; toolName: string; input: string } | null;
  /** The unanswered `AskUserQuestion`, if the session is parked on one. */
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
    model: null,
    claudeSessionId: null,
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

/** True while any permission card is still unanswered. The turn is blocked on
 *  the user, so the session counts as busy even if a stray `turnDone` (e.g. a
 *  subagent finishing) tried to un-park it while sibling asks are outstanding. */
export function hasPendingPermission(view: SessionView): boolean {
  return view.items.some((it) => it.kind === "permission" && it.decision === null);
}

/** Whether the agent is actively working (drives spinners / disables Send). A
 *  pending question blocks the turn on the user, so it counts as busy too. */
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

/**
 * Whether claude is actively *computing* — drives the "time working" elapsed
 * counter and the tab's breathing glow. Deliberately narrower than [`isBusy`]: a
 * session parked on a permission or question is busy (you can't take a new turn)
 * but it is NOT working — it's blocked on you. Treating "waiting on the user" as
 * "working" is what made a restored, permission-held session look like a running
 * turn after an app restart; it also means the counter/glow now correctly pause
 * while a *live* session waits for your approval and resume once you answer.
 */
export function isWorking(view: SessionView): boolean {
  if (isTerminal(view.state)) return false;
  if (view.state === "awaiting_permission" || view.state === "awaiting_question") return false;
  if (hasPendingPermission(view) || view.pendingQuestion !== null) return false;
  return view.state === "starting" || view.streaming;
}

/** The state a non-terminal turn should rest in given outstanding user prompts:
 *  a question takes precedence, then an open permission card, else it's running.
 *  Shared by `turnDone` and the answer/permission resolvers so they stay in sync. */
export function parkedState(view: SessionView): SessionState {
  if (isTerminal(view.state)) return view.state;
  if (view.pendingQuestion !== null) return "awaiting_question";
  if (hasPendingPermission(view)) return "awaiting_permission";
  return "running";
}

/**
 * Finalize a view rebuilt by replaying a session's persisted log after an app
 * restart. The claude child doesn't outlive the app (`kill_on_drop`), so a
 * replayed session is never live — settle it into a consistent resting state:
 *
 *  - already terminal (completed / crashed / killed / …) → leave it;
 *  - parked on a permission or question → keep it HELD (state stays
 *    `awaiting_*`, the card and pending prompt stay) so the conversation pauses
 *    for the user instead of looking like a running turn — answering it resumes
 *    the process (claude re-issues the ask);
 *  - otherwise it was cut off mid-turn → settle to `idle`.
 *
 * `streaming` is always cleared: nothing is actively streaming after a restart.
 * Crucially this never leaves the buggy "idle but an unresolved permission card
 * is still open" shape, which read as busy/working forever.
 */
export function restoredView(view: SessionView): SessionView {
  if (isTerminal(view.state)) return { ...view, streaming: false };
  if (view.pendingQuestion !== null || hasPendingPermission(view)) {
    return { ...view, state: parkedState(view), streaming: false };
  }
  return {
    ...view,
    state: "idle",
    streaming: false,
    pendingPermission: null,
    pendingQuestion: null,
  };
}

/**
 * Settle the prompt that a just-completed main-thread tool was gating. A
 * permission decision and an `AskUserQuestion` answer are both frontend-only
 * actions — neither emits an event of its own — so the tool's *result* is the
 * only thing persisted to the log that proves the user already answered. Without
 * this, replaying the log after a restart re-surfaces an answered card and locks
 * the composer on a stale prompt (the conversation looks paused when it isn't).
 *
 *  - `AskUserQuestion` finishing → the questions were answered → drop the
 *    pending question;
 *  - any other tool finishing → mark its still-open permission card resolved
 *    (matched by tool name — within a turn a tool either always gates or never
 *    does, and answered tools' results land before any later ask, so this can't
 *    clear a genuinely-pending card).
 *
 * Idempotent with the live resolvers, which already cleared the prompt before
 * the result arrives; on replay this is the mechanism that clears it.
 */
function settleAnsweredPrompt(
  view: SessionView,
  items: TranscriptItem[],
  toolName: string,
): SessionView {
  let nextItems = items;
  let pendingPermission = view.pendingPermission;
  let pendingQuestion = view.pendingQuestion;

  if (toolName === "AskUserQuestion") {
    pendingQuestion = null;
  } else {
    for (let i = nextItems.length - 1; i >= 0; i -= 1) {
      const it = nextItems[i];
      if (it.kind === "permission" && it.decision === null && it.toolName === toolName) {
        nextItems = nextItems.slice();
        nextItems[i] = { ...it, decision: "allow" };
        if (pendingPermission?.requestId === it.requestId) pendingPermission = null;
        break;
      }
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

/* ------------------------------- internals ------------------------------- */

/**
 * Index of the open main-thread streaming assistant block, or -1. Scans from
 * the end (not just the last item) so it's still found when subagent steps were
 * appended after it — the main agent streams progress text while parallel
 * subagents run, interleaving their events with its partials.
 */
function openAssistantIndex(items: TranscriptItem[]): number {
  for (let i = items.length - 1; i >= 0; i -= 1) {
    const it = items[i];
    if (it.kind === "assistant" && it.streaming && !it.parentId) return i;
  }
  return -1;
}

/** The earliest `Skill` tool row in the same thread (`parentId`) still awaiting
 *  its injected body, so a skill-body block folds into its invocation row rather
 *  than repeating the header as a separate block. Scans front-to-back so that if
 *  several skills are queued, bodies attach in invocation (FIFO) order. -1 if
 *  none. */
function firstSkillToolAwaitingBody(items: TranscriptItem[], parentId: string | null): number {
  for (let i = 0; i < items.length; i += 1) {
    const it = items[i];
    if (
      it.kind === "tool" &&
      it.name === "Skill" &&
      it.skillBody === undefined &&
      (it.parentId ?? null) === parentId
    ) {
      return i;
    }
  }
  return -1;
}

/** The earliest `compact` divider still awaiting its post-compact summary, so the
 *  summary block folds into it rather than dumping inline. A boundary is followed
 *  immediately by its summary, so in practice only one is ever awaiting; -1 if
 *  none (then the block is treated as ordinary prose). */
function firstCompactAwaitingSummary(items: TranscriptItem[]): number {
  for (let i = 0; i < items.length; i += 1) {
    const it = items[i];
    if (it.kind === "compact" && it.summary === undefined) return i;
  }
  return -1;
}

/** Mark the open streaming assistant block (if any) as finished. */
function closeOpenAssistant(items: TranscriptItem[]): TranscriptItem[] {
  const idx = openAssistantIndex(items);
  if (idx === -1) return items;
  const block = items[idx] as Extract<TranscriptItem, { kind: "assistant" }>;
  const copy = items.slice();
  copy[idx] = { ...block, streaming: false };
  return copy;
}

/**
 * Append a freshly-built item, allocating its deterministic id and advancing the
 * counter — the close/allocate/append/increment plumbing every appending case
 * shares. `closeAssistant` first finalizes the open streaming block (main-thread
 * items do this; subagent items just nest). Returns the `items`/`counter` slice
 * to spread; each case keeps its own `streaming`/`state` semantics explicit.
 */
function appended(
  view: SessionView,
  make: (id: string) => TranscriptItem,
  closeAssistant = false,
): Pick<SessionView, "items" | "counter"> {
  const base = closeAssistant ? closeOpenAssistant(view.items) : view.items;
  return {
    items: [...base, make(`i${view.counter}`)],
    counter: view.counter + 1,
  };
}

/* -------------------------------- reducer -------------------------------- */

export function reduceEvent(view: SessionView, event: SessionEvent): SessionView {
  const next = reduceEventInner(view, event);
  // "starting" means: spawned, no output yet — the "Connecting to <harness>…"
  // state. The first sign of life (any event that leaves a turn streaming) means
  // the harness is up and producing, so leave the connecting state even if we never
  // saw an `init`: a live session re-tailed after a UI/HMR reload doesn't
  // re-emit one, which otherwise pinned the spinner on forever while the turn
  // ran fine. `init`/permission/question/turnDone/exit already set their own
  // state, so this only ever promotes a genuinely-stuck "starting".
  if (next.state === "starting" && next.streaming) {
    return { ...next, state: "running" };
  }
  return next;
}

function reduceEventInner(view: SessionView, event: SessionEvent): SessionView {
  switch (event.kind) {
    case "init":
      return {
        ...view,
        state: "running",
        streaming: true,
        claudeSessionId: event.claudeSessionId || view.claudeSessionId,
        model: event.model ?? view.model,
      };

    case "userMessage":
      // Persisted user turn, seen only when replaying the log on restart (the
      // backend doesn't forward it live — the optimistic `addUserMessage` already
      // showed it). Rebuilds the user's side of the conversation. `createdAt` is
      // null here and backfilled by the replay loop from the stored event's
      // `createdAt` column (the reducer is pure and clock-free).
      return {
        ...view,
        ...appended(
          view,
          (id) => ({
            kind: "user",
            id,
            text: event.text,
            stage: event.stage,
            attachments: [],
            createdAt: null,
          }),
          true,
        ),
        streaming: true,
      };

    case "partialText": {
      // Subagent streaming is noise in the main thread; the completed subagent
      // blocks get nested under the Task instead.
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
        ...appended(view, (id) => ({ kind: "assistant", id, text: event.text, streaming: true })),
        streaming: true,
      };
    }

    case "assistantText": {
      const skill = skillBodyName(event.text);
      if (skill) {
        // A launched skill's injected SKILL.md body. In practice it always
        // follows the skill's own invocation row, so fold the body into that row
        // (expand to read it) rather than repeat the header as a second block.
        const toolIdx = firstSkillToolAwaitingBody(view.items, event.parentId ?? null);
        if (toolIdx !== -1) {
          const tool = view.items[toolIdx] as Extract<TranscriptItem, { kind: "tool" }>;
          const items = view.items.slice();
          items[toolIdx] = { ...tool, skillBody: event.text };
          return { ...view, items, streaming: true };
        }
        // No invocation row to fold into (rare) — keep the body as its own
        // collapsed chip so it's still inspectable but never dumped inline. If it
        // streamed into an open assistant block first, convert that in place.
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
          ...appended(view, makeSkill, !event.parentId),
          streaming: true,
        };
      }
      // claude's post-compact summary — a single, often 10–20KB assistant block
      // injected right after the compact boundary. Fold it into that divider
      // (expand to read it) instead of dumping the whole summary inline. Main
      // thread only, like the boundary; and only when a boundary is actually
      // awaiting its summary, so a coincidental prefix match with no recent
      // compaction stays ordinary prose.
      if (!event.parentId && isCompactSummary(event.text)) {
        // The summary normally lands whole, but if it streamed into an open
        // assistant block first, drop that partial so it isn't shown twice.
        const open = openAssistantIndex(view.items);
        const base = open === -1 ? view.items : view.items.filter((_, i) => i !== open);
        const compactIdx = firstCompactAwaitingSummary(base);
        if (compactIdx !== -1) {
          const compact = base[compactIdx] as Extract<TranscriptItem, { kind: "compact" }>;
          const items = base.slice();
          items[compactIdx] = { ...compact, summary: event.text };
          return { ...view, items, streaming: true };
        }
      }
      // Subagent text nests under its Task — append, never touch the main block.
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
          })),
          streaming: true,
        };
      }
      const idx = openAssistantIndex(view.items);
      if (idx !== -1) {
        // The completed block is authoritative; fall back to the streamed text
        // only if claude sent an empty final (it occasionally does).
        const block = view.items[idx] as Extract<TranscriptItem, { kind: "assistant" }>;
        const items = view.items.slice();
        items[idx] = {
          ...block,
          text: event.text.length ? event.text : block.text,
          streaming: false,
        };
        return { ...view, items, streaming: true };
      }
      if (!event.text.length) return view; // empty block, nothing open → ignore
      return {
        ...view,
        ...appended(view, (id) => ({ kind: "assistant", id, text: event.text, streaming: false })),
        streaming: true,
      };
    }

    case "thinking": {
      if (!event.text.length) return view;
      // Main-thread blocks close the open assistant; subagent ones just nest.
      return {
        ...view,
        ...appended(
          view,
          (id) => ({ kind: "thinking", id, text: event.text, parentId: event.parentId }),
          !event.parentId,
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
        ),
        streaming: true,
      };
    }

    case "toolResult": {
      // Attach to the matching open tool block (search from the end).
      for (let i = view.items.length - 1; i >= 0; i -= 1) {
        const item = view.items[i];
        if (item.kind === "tool" && item.toolUseId === event.toolUseId && item.result === null) {
          const items = view.items.slice();
          items[i] = { ...item, result: { content: event.content, isError: event.isError } };
          // A finished main-thread tool is our persisted proof that the prompt
          // gating it was answered — settle it so a replay doesn't re-surface an
          // already-answered card. (Subagent results gate nothing on the main
          // thread.)
          return event.parentId
            ? { ...view, items, streaming: true }
            : settleAnsweredPrompt(view, items, item.name);
        }
      }
      // No matching tool_use (rare): surface the orphan result on its own.
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
        })),
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
            decision: null,
          }),
          true,
        ),
        state: "awaiting_permission",
        streaming: true,
        pendingPermission: {
          requestId: event.requestId,
          toolName: event.toolName,
          input: event.input,
        },
      };
    }

    case "questionAsk": {
      // The matching `AskUserQuestion` tool block (its toolUse) is already in the
      // transcript; the questions render interactively above the composer, so we
      // only park the session and stash them — no transcript item to add here.
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
      // A live context reading mid-turn — update the meter, touch nothing else.
      return { ...view, contextTokens: event.contextTokens };

    case "turnDone": {
      // A turnDone can arrive (e.g. a subagent finishing) while the user still
      // owes an answer to a question or an outstanding permission ask — the real
      // turn isn't done. Stay parked rather than dropping to "running" and
      // looking idle.
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

    case "exited":
      return {
        ...view,
        ...appended(
          view,
          (id) => ({
            kind: "exit",
            id,
            outcome: event.outcome,
            code: event.code,
            stderrTail: event.stderrTail,
          }),
          true,
        ),
        state: event.outcome,
        streaming: false,
        pendingPermission: null,
        pendingQuestion: null,
      };

    case "protocolError":
      return {
        ...view,
        ...appended(view, (id) => ({ kind: "error", id, detail: event.detail, raw: event.raw })),
      };

    case "compactBoundary":
      // claude paused to summarize the conversation. The next assistant turn
      // resumes on the post-compact summary; close any open prose block so the
      // divider doesn't land mid-stream. Busy state is unchanged: a turn that
      // was in flight stays in flight until its own `result`/`turnDone` lands.
      return {
        ...view,
        ...appended(
          view,
          (id) => ({
            kind: "compact",
            id,
            trigger: event.trigger,
            preTokens: event.preTokens,
          }),
          true,
        ),
      };

    case "relaunched":
      // Inert boundary marker; the new process's own `init` (next) sets state.
      return {
        ...view,
        ...appended(view, (id) => ({ kind: "relaunched", id })),
      };

    case "backgroundTaskUpdate":
      // An out-of-band async signal: claude fires it the moment a background
      // shell finishes, which can land between turns (it even wakes an
      // autonomous turn to react). Append a non-rendering marker that
      // `backgroundTasksFromItems` consumes to drop the finished task; leave
      // streaming/state untouched so it can't perturb an in-flight or resting
      // turn, and don't close the open assistant block (it isn't turn content).
      return {
        ...view,
        ...appended(view, (id) => ({
          kind: "taskUpdate",
          id,
          taskId: event.taskId,
          toolUseId: event.toolUseId,
          status: event.status,
        })),
      };

    default:
      return view;
  }
}

/* ---------------------------- frontend actions --------------------------- */

/** Append the user's turn optimistically and flag a turn as in flight. The
 *  caller passes `createdAt` (epoch ms) so the reducer stays clock-free and
 *  tests stay deterministic — the live path passes `Date.now()`. */
export function addUserMessage(
  view: SessionView,
  text: string,
  stage: string | null,
  attachments: Attachment[] = [],
  createdAt: number | null = null,
): SessionView {
  return {
    ...view,
    ...appended(
      view,
      (id) => ({ kind: "user", id, text, stage, attachments, createdAt }),
      true,
    ),
    streaming: true,
  };
}

/**
 * Apply the user's answer to a permission ask (there's no backend event for
 * this — the driver just resumes). Marks the card resolved and un-parks the
 * session so the turn can continue.
 */
export function resolvePermission(
  view: SessionView,
  requestId: string,
  decision: PermissionDecision,
): SessionView {
  const items = view.items.map((item) =>
    item.kind === "permission" && item.requestId === requestId && item.decision === null
      ? { ...item, decision }
      : item,
  );
  // A subagent can have several asks open at once; only resume the turn once the
  // last card is answered (and no question is outstanding) — until then the
  // session is still parked (see `parkedState`).
  const pendingPermission =
    view.pendingPermission?.requestId === requestId ? null : view.pendingPermission;
  const next = { ...view, items, pendingPermission };
  return { ...next, state: parkedState(next), streaming: true };
}

/**
 * Apply the user's answer to an `AskUserQuestion` prompt (no backend event —
 * the driver just resumes once the answer is sent). Clears the pending question
 * and un-parks the session so the turn can continue.
 */
export function resolveQuestion(view: SessionView, requestId: string): SessionView {
  if (view.pendingQuestion?.requestId !== requestId) return view;
  const next = { ...view, pendingQuestion: null };
  return { ...next, state: parkedState(next), streaming: true };
}

/**
 * Cancel every still-open permission card and any pending question — for a turn
 * that's being *abandoned* wholesale rather than answered (a permission-mode
 * restart kills the asking process; Stop cancels the ask outright). Open cards
 * are marked `deny` (the tool never ran) and both pending prompts cleared, so the
 * session can't stay `isBusy` — composer locked, queue frozen — parked on a
 * prompt that no live claude will ever resolve (its request id died with the old
 * process, or the user chose to stop instead of answering). The caller settles
 * `state`/`streaming`. Idempotent: a view with nothing parked is returned as-is.
 */
export function cancelPendingPrompts(view: SessionView): SessionView {
  const items = view.items.map((it) =>
    it.kind === "permission" && it.decision === null ? { ...it, decision: "deny" as const } : it,
  );
  return { ...view, items, pendingPermission: null, pendingQuestion: null };
}

/* -------------------------------- display -------------------------------- */

export type StatusTone = "ok" | "warn" | "bad" | "dim" | "accent";

/** Human label + badge tone for a session state (streaming refines "running"). */
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
  spawn_failed: "Failed to start session",
};

export function exitLabel(outcome: ExitOutcome): string {
  return EXIT_LABEL[outcome] ?? "Session ended";
}

/** Flatten a transcript to plain text (for "Copy transcript"). */
export function viewToText(view: SessionView): string {
  const lines: string[] = [];
  for (const item of view.items) {
    switch (item.kind) {
      case "user": {
        const heading = `## You${item.stage && item.stage !== "general" ? ` (${item.stage})` : ""}`;
        lines.push(heading, item.text);
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
        // Name the loaded skill, not its full injected body (which the chip keeps collapsed).
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
        // Mark the boundary; the summary itself stays out of the copy (it's the
        // bulky injected block the divider keeps collapsed), like skill bodies.
        lines.push("--- Conversation compacted ---", "");
        break;
      case "exit":
        lines.push(`--- ${exitLabel(item.outcome)} ---`, "");
        break;
    }
  }
  return lines.join("\n").trim();
}
