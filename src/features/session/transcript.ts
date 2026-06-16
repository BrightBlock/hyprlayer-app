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

/** Set on items produced inside a subagent (the `Task` tool-use id); the UI
 *  nests them under that Task instead of inlining them in the main thread. */
type Parented = { parentId?: string | null };

export type TranscriptItem =
  | { kind: "user"; id: string; text: string; stage: string | null }
  | ({ kind: "assistant"; id: string; text: string; streaming: boolean } & Parented)
  | ({ kind: "thinking"; id: string; text: string } & Parented)
  | ({
      kind: "tool";
      id: string;
      toolUseId: string;
      name: string;
      input: string;
      result: { content: string; isError: boolean } | null;
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

/** The state a non-terminal turn should rest in given outstanding user prompts:
 *  a question takes precedence, then an open permission card, else it's running.
 *  Shared by `turnDone` and the answer/permission resolvers so they stay in sync. */
function parkedState(view: SessionView): SessionState {
  if (isTerminal(view.state)) return view.state;
  if (view.pendingQuestion !== null) return "awaiting_question";
  if (hasPendingPermission(view)) return "awaiting_permission";
  return "running";
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
  switch (event.kind) {
    case "init":
      return {
        ...view,
        state: "running",
        streaming: true,
        claudeSessionId: event.claudeSessionId || view.claudeSessionId,
        model: event.model ?? view.model,
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
          return { ...view, items, streaming: true };
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

    default:
      return view;
  }
}

/* ---------------------------- frontend actions --------------------------- */

/** Append the user's turn optimistically and flag a turn as in flight. */
export function addUserMessage(
  view: SessionView,
  text: string,
  stage: string | null,
): SessionView {
  return {
    ...view,
    ...appended(view, (id) => ({ kind: "user", id, text, stage }), true),
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
      case "user":
        lines.push(`## You${item.stage && item.stage !== "general" ? ` (${item.stage})` : ""}`, item.text, "");
        break;
      case "assistant":
        lines.push("## Assistant", item.text, "");
        break;
      case "thinking":
        lines.push("> [thinking]", ...item.text.split("\n").map((l) => `> ${l}`), "");
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
      case "exit":
        lines.push(`--- ${exitLabel(item.outcome)} ---`, "");
        break;
    }
  }
  return lines.join("\n").trim();
}
