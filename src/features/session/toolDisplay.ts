/**
 * Pure presentation helpers for agent tool calls. No React, no IPC — just
 * turning the compact-JSON `input` string that rides on a `toolUse` /
 * `permissionAsk` event into something a human can read, the way Claude Code
 * renders a tool call: the tool name, then a concise one-line argument summary
 * (the bash command, the file path, the search pattern, …).
 */

/** Parse a tool's `input` (compact JSON string) into an object, or null. */
export function parseToolInput(input: string): Record<string, unknown> | null {
  if (!input) return null;
  try {
    const value: unknown = JSON.parse(input);
    return value !== null && typeof value === "object" && !Array.isArray(value)
      ? (value as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
}

function str(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return "";
}

/** First whitespace-collapsed line of a value, for compact single-line headers. */
function firstLine(value: string): string {
  const trimmed = value.trim();
  const nl = trimmed.indexOf("\n");
  return nl === -1 ? trimmed : `${trimmed.slice(0, nl).trim()} …`;
}

/**
 * A concise, single-line argument summary for a tool call header — chosen per
 * tool to surface the argument that matters (Claude-Code style). Unknown tools
 * fall back to their first string-valued argument.
 */
export function toolSummary(name: string, input: string): string {
  const obj = parseToolInput(input);
  if (!obj) return "";
  switch (name) {
    case "Bash":
    case "BashOutput":
      return firstLine(str(obj.command));
    case "Read":
    case "Write":
    case "NotebookEdit":
      return str(obj.file_path) || str(obj.path) || str(obj.notebook_path);
    case "Edit":
    case "MultiEdit":
      return str(obj.file_path);
    case "Glob":
      return str(obj.pattern) + (obj.path ? `  (${str(obj.path)})` : "");
    case "Grep": {
      const pattern = str(obj.pattern);
      const path = str(obj.path);
      return path ? `${pattern}  in ${path}` : pattern;
    }
    case "Task":
      return str(obj.description) || str(obj.subagent_type);
    case "WebFetch":
      return str(obj.url);
    case "WebSearch":
      return str(obj.query);
    case "TodoWrite":
      return Array.isArray(obj.todos) ? `${obj.todos.length} items` : "";
    case "AskUserQuestion": {
      const questions = Array.isArray(obj.questions) ? obj.questions : [];
      const first = questions[0] as { question?: unknown } | undefined;
      const text = firstLine(str(first?.question));
      return questions.length > 1 ? `${text} (+${questions.length - 1} more)` : text;
    }
    case "KillBash":
      return str(obj.shell_id);
    default: {
      const firstString = Object.values(obj).find((v) => typeof v === "string");
      return typeof firstString === "string" ? firstLine(firstString) : "";
    }
  }
}

/** Tools whose summary is a single filesystem path the transcript can show as
 *  a compact basename chip instead of the full absolute path. */
const FILE_PATH_TOOLS = new Set(["Read", "Write", "Edit", "MultiEdit", "NotebookEdit"]);

/**
 * For a file-targeting tool, the absolute path it acts on (else null). Lets the
 * transcript render a compact filename chip while keeping the full path on hover
 * and in the expanded view.
 */
export function toolFilePath(name: string, input: string): string | null {
  if (!FILE_PATH_TOOLS.has(name)) return null;
  const obj = parseToolInput(input);
  if (!obj) return null;
  return str(obj.file_path) || str(obj.path) || str(obj.notebook_path) || null;
}

/** Final path segment (`/a/b/c.ts` → `c.ts`), tolerating trailing slashes and
 *  Windows `\` separators (dropped-file paths can use either). */
export function baseName(path: string): string {
  const trimmed = path.replace(/[/\\]+$/, "");
  const cut = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  return trimmed.slice(cut + 1) || trimmed;
}

/**
 * A concise one-line preview of a tool's result, for the dimmed `⎿` line shown
 * under a collapsed tool row. Strips the `N⇥` line-number prefix Read adds and
 * skips blank leading lines so the preview is the first line that says something.
 */
export function resultPreview(content: string, isError: boolean): string {
  const firstMeaningful = content.split("\n").find((line) => line.trim() !== "");
  if (firstMeaningful === undefined) return isError ? "(error)" : "(no output)";
  // `cat -n`-style "  12\tcode" → "code"; leave other content untouched.
  const cleaned = firstMeaningful.replace(/^\s*\d+\t/, "").trim();
  return cleaned || (isError ? "(error)" : "(no output)");
}

/**
 * The expanded view of a tool's input: pretty-printed JSON when it parses,
 * otherwise the raw string (so even a malformed payload is still inspectable).
 */
export function prettyToolInput(input: string): string {
  if (!input) return "";
  try {
    return JSON.stringify(JSON.parse(input), null, 2);
  } catch {
    return input;
  }
}

export type DocKind = "research" | "plan";

/**
 * If a tool call wrote/edited a research or plan doc (a `.md` under a
 * `…/research/…` or `…/plans/…` path), return its path + kind — used to surface
 * agent-created docs in the diff sidebar. Other writes (code in the worktree)
 * return null.
 */
export function docFromTool(name: string, input: string): { path: string; kind: DocKind } | null {
  if (name !== "Write" && name !== "Edit" && name !== "MultiEdit") return null;
  const obj = parseToolInput(input);
  const path = str(obj?.file_path);
  if (!path.endsWith(".md")) return null;
  if (path.includes("/research/")) return { path, kind: "research" };
  if (path.includes("/plans/") || path.includes("/plan/")) return { path, kind: "plan" };
  return null;
}

/** A human title for a doc path: filename without the date prefix / extension. */
export function docTitle(path: string): string {
  const file = path.split("/").pop() ?? path;
  return (
    file
      .replace(/\.md$/i, "")
      .replace(/^\d{4}-\d{2}-\d{2}[-_]?/, "")
      .replace(/[-_]+/g, " ")
      .trim() || file
  );
}

/** Compact token count: 24130 → "24.1k", 1_000_000 → "1.0M", 980 → "980". */
export function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

/** Format a cumulative USD cost like the session footer ($0.0123). */
export function formatCost(usd: number | null | undefined): string | null {
  if (usd === null || usd === undefined || !Number.isFinite(usd)) return null;
  // Sub-cent costs still read as $0.0042; round to 4dp and trim nothing.
  return `$${usd.toFixed(usd < 1 ? 4 : 2)}`;
}

/**
 * A live "time working" label for the elapsed counter (Conductor-style): tenths
 * of a second under a minute (19.3s), then minutes + zero-padded seconds
 * (1m 23s), then hours + minutes for a rare long run (1h 04m). Negative inputs
 * clamp to zero.
 */
export function formatElapsed(ms: number): string {
  const totalSec = Math.max(0, ms) / 1000;
  if (totalSec < 60) return `${totalSec.toFixed(1)}s`;
  const totalMin = Math.floor(totalSec / 60);
  if (totalMin < 60) {
    return `${totalMin}m ${String(Math.floor(totalSec % 60)).padStart(2, "0")}s`;
  }
  return `${Math.floor(totalMin / 60)}h ${String(totalMin % 60).padStart(2, "0")}m`;
}
