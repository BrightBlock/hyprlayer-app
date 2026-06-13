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
  | ExitOutcome;

export type PermissionDecision = "allow" | "deny";

export type TranscriptItem =
  | { kind: "user"; id: string; text: string; stage: string | null }
  | { kind: "assistant"; id: string; text: string; streaming: boolean }
  | { kind: "thinking"; id: string; text: string }
  | {
      kind: "tool";
      id: string;
      toolUseId: string;
      name: string;
      input: string;
      result: { content: string; isError: boolean } | null;
    }
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
  model: string | null;
  claudeSessionId: string | null;
  /** The unanswered permission ask, if the session is parked on one. */
  pendingPermission: { requestId: string; toolName: string; input: string } | null;
};

export function initialSessionView(state: SessionState = "idle"): SessionView {
  return {
    items: [],
    counter: 0,
    state,
    streaming: false,
    costUsd: null,
    numTurns: null,
    model: null,
    claudeSessionId: null,
    pendingPermission: null,
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

/** Whether the agent is actively working (drives spinners / disables Send). */
export function isBusy(view: SessionView): boolean {
  if (isTerminal(view.state)) return false;
  return view.state === "starting" || view.state === "awaiting_permission" || view.streaming;
}

/* ------------------------------- internals ------------------------------- */

/** Mark the trailing still-streaming assistant block (if any) as finished. */
function closeOpenAssistant(items: TranscriptItem[]): TranscriptItem[] {
  const last = items[items.length - 1];
  if (last && last.kind === "assistant" && last.streaming) {
    const copy = items.slice();
    copy[copy.length - 1] = { ...last, streaming: false };
    return copy;
  }
  return items;
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
      if (!event.text) return view;
      const last = view.items[view.items.length - 1];
      if (last && last.kind === "assistant" && last.streaming) {
        const items = view.items.slice();
        items[items.length - 1] = { ...last, text: last.text + event.text };
        return { ...view, items, streaming: true };
      }
      const id = `i${view.counter}`;
      return {
        ...view,
        items: [...view.items, { kind: "assistant", id, text: event.text, streaming: true }],
        counter: view.counter + 1,
        streaming: true,
      };
    }

    case "assistantText": {
      const last = view.items[view.items.length - 1];
      if (last && last.kind === "assistant" && last.streaming) {
        // The completed block is authoritative; fall back to the streamed text
        // only if claude sent an empty final (it occasionally does).
        const items = view.items.slice();
        items[items.length - 1] = {
          ...last,
          text: event.text.length ? event.text : last.text,
          streaming: false,
        };
        return { ...view, items, streaming: true };
      }
      if (!event.text.length) return view; // empty block, nothing open → ignore
      const id = `i${view.counter}`;
      return {
        ...view,
        items: [...view.items, { kind: "assistant", id, text: event.text, streaming: false }],
        counter: view.counter + 1,
        streaming: true,
      };
    }

    case "thinking": {
      if (!event.text.length) return view;
      const items = closeOpenAssistant(view.items);
      const id = `i${view.counter}`;
      return {
        ...view,
        items: [...items, { kind: "thinking", id, text: event.text }],
        counter: view.counter + 1,
        streaming: true,
      };
    }

    case "toolUse": {
      const items = closeOpenAssistant(view.items);
      const id = `i${view.counter}`;
      return {
        ...view,
        items: [
          ...items,
          {
            kind: "tool",
            id,
            toolUseId: event.id,
            name: event.name,
            input: event.input,
            result: null,
          },
        ],
        counter: view.counter + 1,
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
      const id = `i${view.counter}`;
      return {
        ...view,
        items: [
          ...view.items,
          {
            kind: "tool",
            id,
            toolUseId: event.toolUseId,
            name: "result",
            input: "",
            result: { content: event.content, isError: event.isError },
          },
        ],
        counter: view.counter + 1,
        streaming: true,
      };
    }

    case "permissionAsk": {
      const items = closeOpenAssistant(view.items);
      const id = `i${view.counter}`;
      return {
        ...view,
        items: [
          ...items,
          {
            kind: "permission",
            id,
            requestId: event.requestId,
            toolName: event.toolName,
            input: event.input,
            decision: null,
          },
        ],
        counter: view.counter + 1,
        state: "awaiting_permission",
        streaming: true,
        pendingPermission: {
          requestId: event.requestId,
          toolName: event.toolName,
          input: event.input,
        },
      };
    }

    case "turnDone":
      return {
        ...view,
        items: closeOpenAssistant(view.items),
        state: isTerminal(view.state) ? view.state : "running",
        streaming: false,
        costUsd: event.costUsd ?? view.costUsd,
        numTurns: event.numTurns ?? view.numTurns,
      };

    case "exited": {
      const id = `i${view.counter}`;
      return {
        ...view,
        items: [
          ...closeOpenAssistant(view.items),
          {
            kind: "exit",
            id,
            outcome: event.outcome,
            code: event.code,
            stderrTail: event.stderrTail,
          },
        ],
        counter: view.counter + 1,
        state: event.outcome,
        streaming: false,
        pendingPermission: null,
      };
    }

    case "protocolError": {
      const id = `i${view.counter}`;
      return {
        ...view,
        items: [...view.items, { kind: "error", id, detail: event.detail, raw: event.raw }],
        counter: view.counter + 1,
      };
    }

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
  const items = closeOpenAssistant(view.items);
  const id = `i${view.counter}`;
  return {
    ...view,
    items: [...items, { kind: "user", id, text, stage }],
    counter: view.counter + 1,
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
  const stillPending = view.pendingPermission?.requestId === requestId ? null : view.pendingPermission;
  return {
    ...view,
    items,
    pendingPermission: stillPending,
    state: isTerminal(view.state) ? view.state : "running",
    streaming: true,
  };
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
