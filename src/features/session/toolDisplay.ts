/**
 * Pure presentation helpers for agent tool calls. No React, no IPC — just
 * turning the compact-JSON `input` string that rides on a `toolUse` /
 * `permissionAsk` event into something a human can read, the way Claude Code
 * renders a tool call: the tool name, then a concise one-line argument summary
 * (the bash command, the file path, the search pattern, …).
 */

import { diffStrings, type PatchLine } from "../workstreams/diff";

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
    case "Skill": {
      // Input is `{skill, args?}`, but Claude Code emits the keys in `args`-first
      // order — so the generic "first string arg" fallback would surface the args
      // (often a bare number like a PR id) instead of the skill being invoked.
      const skill = str(obj.skill);
      const skillArgs = firstLine(str(obj.args));
      return skillArgs ? `${skill}  ${skillArgs}` : skill;
    }
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

export type EditDiff = { lines: PatchLine[]; additions: number; deletions: number };

/**
 * The line-level diff an Edit / MultiEdit tool call applies, derived from its
 * `old_string` / `new_string` payload — lets the transcript preview the change on
 * hover instead of just naming the file. MultiEdit's edits are stacked, separated
 * by a hunk marker. Returns null for other tools or an unparseable / empty input.
 */
export function editDiffFromTool(name: string, input: string): EditDiff | null {
  if (name !== "Edit" && name !== "MultiEdit") return null;
  const obj = parseToolInput(input);
  if (!obj) return null;

  const pairs: Array<{ oldText: string; newText: string }> = [];
  if (name === "Edit") {
    pairs.push({ oldText: str(obj.old_string), newText: str(obj.new_string) });
  } else if (Array.isArray(obj.edits)) {
    for (const edit of obj.edits) {
      if (edit && typeof edit === "object") {
        const rec = edit as Record<string, unknown>;
        pairs.push({ oldText: str(rec.old_string), newText: str(rec.new_string) });
      }
    }
  }
  if (pairs.length === 0) return null;

  const lines: PatchLine[] = [];
  pairs.forEach((pair, index) => {
    if (index > 0) lines.push({ kind: "hunk", text: "@@" });
    lines.push(...diffStrings(pair.oldText, pair.newText));
  });
  if (lines.length === 0) return null;

  let additions = 0;
  let deletions = 0;
  for (const line of lines) {
    if (line.kind === "add") additions += 1;
    else if (line.kind === "del") deletions += 1;
  }
  return { lines, additions, deletions };
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

/**
 * A short relative-time label for a past instant, Claude-Code-style:
 * `just now`, `15s ago`, `4m ago`, `2h ago`, `3d ago`, `2w ago`, `5mo ago`,
 * `1y ago`. `from` defaults to a fresh `Date.now()` reading; pass it explicitly
 * in tests for determinism. Falls back to `just now` for the future or for
 * invalid values (a backfilled-from-null timestamp).
 */
export function formatRelativeTime(epochMs: number | null, from: number = Date.now()): string {
  if (epochMs === null || !Number.isFinite(epochMs)) return "just now";
  const seconds = Math.max(0, Math.floor((from - epochMs) / 1000));
  if (seconds < 10) return "just now";
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d ago`;
  const weeks = Math.floor(days / 7);
  if (weeks < 5) return `${weeks}w ago`;
  const months = Math.floor(days / 30);
  if (months < 12) return `${months}mo ago`;
  const years = Math.floor(days / 365);
  return `${years}y ago`;
}
