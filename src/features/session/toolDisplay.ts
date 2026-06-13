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
    case "KillBash":
      return str(obj.shell_id);
    default: {
      const firstString = Object.values(obj).find((v) => typeof v === "string");
      return typeof firstString === "string" ? firstLine(firstString) : "";
    }
  }
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

/** Format a cumulative USD cost like the session footer ($0.0123). */
export function formatCost(usd: number | null | undefined): string | null {
  if (usd === null || usd === undefined || !Number.isFinite(usd)) return null;
  // Sub-cent costs still read as $0.0042; round to 4dp and trim nothing.
  return `$${usd.toFixed(usd < 1 ? 4 : 2)}`;
}
