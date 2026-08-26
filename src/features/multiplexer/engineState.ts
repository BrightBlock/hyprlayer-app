
export type AgentState = "working" | "blocked" | "done" | "idle";

// eslint-disable-next-line no-control-regex
const ANSI_RE = /[\x1b\x9b][[\]()#;?]*(?:(?:[a-zA-Z\d]*(?:;[a-zA-Z\d]*)*)?\x07|(?:\d{1,4}(?:;\d{0,4})*)?[\dA-PR-TZcf-nq-uy=><~])/g;
// eslint-disable-next-line no-control-regex
const OTHER_CTRL_RE = /[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g;

export function stripAnsi(input: string): string {
  return input
    .replace(ANSI_RE, "")
    .replace(/\r\n/g, "\n")
    .replace(/\r/g, "\n")
    .replace(OTHER_CTRL_RE, "");
}

export function foot(text: string, lines = 14): string {
  const arr = text.split("\n").map((l) => l.trimEnd());
  while (arr.length && arr[arr.length - 1] === "") arr.pop();
  return arr.slice(-lines).join("\n");
}

const BLOCKED_PATTERNS: Record<string, RegExp[]> = {
  "claude-code": [
    /do you want to (proceed|continue|make|create|run|allow|apply)/i,
    /❯?\s*1\.\s*yes\b/i,
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

const GENERIC_BLOCKED: RegExp[] = [
  /\(\s*y\s*\/\s*n\s*\)/i,
  /\[\s*y\s*\/\s*n\s*\]/i,
  /\(\s*yes\s*\/\s*no\s*\)/i,
  /\by\/n\b/i,
];

const READY_PROMPT_RE = /[$%#❯➜»>]\s*$/;

function matchesBlocked(engineId: string, footText: string): boolean {
  const engine = BLOCKED_PATTERNS[engineId] ?? [];
  if (engine.some((re) => re.test(footText))) return true;
  const tail2 = footText.split("\n").slice(-2).join("\n");
  return GENERIC_BLOCKED.some((re) => re.test(tail2));
}

export function detectState(
  engineId: string,
  tail: string,
  quiet: boolean,
  prev: AgentState,
): AgentState {
  const footText = foot(tail);

  if (matchesBlocked(engineId, footText)) return "blocked";

  if (!quiet) return "working";

  const lastLine = footText.split("\n").pop()?.trim() ?? "";
  if (lastLine.length <= 80 && lastLine.endsWith("?") && !READY_PROMPT_RE.test(lastLine)) {
    return "blocked";
  }

  if (prev === "working" || prev === "blocked") return "done";
  return prev === "done" ? "done" : "idle";
}
