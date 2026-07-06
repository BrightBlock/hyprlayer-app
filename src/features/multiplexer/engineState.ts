//! Maps a PTY engine's recent output to a semantic agent state. Multiplexer
//! agents are opaque CLIs (we see only terminal bytes), so state is inferred from
//! the (ANSI-stripped) output tail plus a caller-supplied "gone quiet?" signal:
//!   working — output flowing / a spinner is up.
//!   blocked — a permission gate or question on screen, waiting on the user.
//!   done    — was working/blocked, then went quiet (turn finished).
//!   idle    — spawned with no task yet (a REPL/shell at rest).
//! `blocked` badges the tab + workstream and fires a notification, so its patterns
//! are engine-specific (+ a generic fallback) to keep false positives low.

export type AgentState = "working" | "blocked" | "done" | "idle";

// ANSI/VT control sequences (CSI, OSC, …), stripped so heuristics run on
// readable text. Hex escapes only — no raw control bytes in source.
// eslint-disable-next-line no-control-regex
const ANSI_RE = /[\x1b\x9b][[\]()#;?]*(?:(?:[a-zA-Z\d]*(?:;[a-zA-Z\d]*)*)?\x07|(?:\d{1,4}(?:;\d{0,4})*)?[\dA-PR-TZcf-nq-uy=><~])/g;
// Stray C0 controls, minus tab/newline/CR (\x09/\x0a/\x0d), handled separately below.
// eslint-disable-next-line no-control-regex
const OTHER_CTRL_RE = /[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g;

/** Strip ANSI escapes + stray control bytes; fold carriage returns to newlines
 *  so spinner redraws become lines we can scan rather than one churning line. */
export function stripAnsi(input: string): string {
  return input
    .replace(ANSI_RE, "")
    .replace(/\r\n/g, "\n")
    .replace(/\r/g, "\n")
    .replace(OTHER_CTRL_RE, "");
}

/** The last `lines` non-empty lines of `text` — the visible "foot" of output,
 *  where any current prompt lives. Avoids matching stale scrollback. */
export function foot(text: string, lines = 14): string {
  const arr = text.split("\n").map((l) => l.trimEnd());
  while (arr.length && arr[arr.length - 1] === "") arr.pop();
  return arr.slice(-lines).join("\n");
}

/** Per-engine "the agent is waiting on you" patterns, matched against the foot. */
const BLOCKED_PATTERNS: Record<string, RegExp[]> = {
  "claude-code": [
    /do you want to (proceed|continue|make|create|run|allow|apply)/i,
    /❯?\s*1\.\s*yes\b/i, // the numbered yes / no permission menu
    /press\s+enter\s+to\s+(continue|confirm)/i,
    /would you like to proceed\??/i,
  ],
  codex: [
    /approve (this )?(shell )?command/i,
    /allow (this )?command/i,
    /run this command\??/i,
    /\ballow\b.*\?\s*$/im,
  ],
  gemini: [
    /apply this change\??/i,
    /waiting for (your )?(confirmation|approval)/i,
    /do you approve\??/i,
  ],
};

// Generic "awaiting a yes/no / confirmation" markers, tried for every engine.
const GENERIC_BLOCKED: RegExp[] = [
  /\(\s*y\s*\/\s*n\s*\)/i,
  /\[\s*y\s*\/\s*n\s*\]/i,
  /\(\s*yes\s*\/\s*no\s*\)/i,
  /\by\/n\b/i,
];

// Trailing characters of a prompt that's *ready* (idle/done), not blocked.
const READY_PROMPT_RE = /[$%#❯➜»>]\s*$/;

function matchesBlocked(engineId: string, footText: string): boolean {
  const engine = BLOCKED_PATTERNS[engineId] ?? [];
  if (engine.some((re) => re.test(footText))) return true;
  // Generic y/n markers only on the live prompt (last 2 lines), so a "y/n" or
  // "[y/N]" sitting in earlier tool output / help text can't trip a false block.
  const tail2 = footText.split("\n").slice(-2).join("\n");
  return GENERIC_BLOCKED.some((re) => re.test(tail2));
}

/**
 * Compute the agent's state from its output tail and a quiet signal.
 *
 * Pure + deterministic so it can be unit-tested and coalesced. Priority:
 *  1. a blocked prompt on screen wins (even mid-stream),
 *  2. live output → working,
 *  3. gone quiet after working/blocked → done,
 *  4. gone quiet with a trailing question → blocked (generic fallback),
 *  5. otherwise keep resting state (idle on a fresh spawn).
 */
export function detectState(
  engineId: string,
  tail: string,
  quiet: boolean,
  prev: AgentState,
): AgentState {
  const footText = foot(tail);

  if (matchesBlocked(engineId, footText)) return "blocked";

  if (!quiet) return "working";

  // Quiet now. A short trailing question (no ready-prompt char) reads as waiting
  // on an answer — treat as blocked even without an engine pattern. The length
  // cap keeps a long done-message that merely ends in "?" from false-blocking.
  const lastLine = footText.split("\n").pop()?.trim() ?? "";
  if (lastLine.length <= 80 && lastLine.endsWith("?") && !READY_PROMPT_RE.test(lastLine)) {
    return "blocked";
  }

  if (prev === "working" || prev === "blocked") return "done";
  return prev === "done" ? "done" : "idle";
}
