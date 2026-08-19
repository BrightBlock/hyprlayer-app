/**
 * Pure presentation helpers for agent tool calls (no React/IPC): turn the
 * compact-JSON `input` string from a `toolUse`/`permissionAsk` event into a
 * Claude-Code-style tool name + one-line argument summary.
 */

import type { DocKind } from "@/lib/domain/docs";
import { diffStrings, type PatchLine } from "@/features/workstreams/diff";

import type { WrittenDocs } from "./writtenDocs";

// A tool's `input` is re-parsed many times per render; cache by input string
// (bounded so a long session can't grow it without limit).
const parsedInputCache = new Map<string, Record<string, unknown> | null>();
const PARSED_CACHE_MAX = 1000;

/** Parse a tool's `input` (compact JSON string) into an object, or null. */
export function parseToolInput(input: string): Record<string, unknown> | null {
  if (!input) return null;
  const cached = parsedInputCache.get(input);
  if (cached !== undefined) return cached;
  let result: Record<string, unknown> | null;
  try {
    const value: unknown = JSON.parse(input);
    result =
      value !== null && typeof value === "object" && !Array.isArray(value)
        ? (value as Record<string, unknown>)
        : null;
  } catch {
    result = null;
  }
  if (parsedInputCache.size >= PARSED_CACHE_MAX) {
    // Evict the oldest entry (Map preserves insertion order) to stay bounded.
    const oldest = parsedInputCache.keys().next().value;
    if (oldest !== undefined) parsedInputCache.delete(oldest);
  }
  parsedInputCache.set(input, result);
  return result;
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
 * Concise one-line argument summary for a tool call header, chosen per tool
 * (Claude-Code style). Unknown tools fall back to their first string arg.
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
      // Claude Code emits `{skill, args?}` args-first, so the generic first-string
      // fallback would surface args (often a bare PR id) instead of the skill.
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

/** Tools whose summary is a single path, shown as a compact basename chip. */
const FILE_PATH_TOOLS = new Set(["Read", "Write", "Edit", "MultiEdit", "NotebookEdit"]);

/** For a file-targeting tool, the absolute path it acts on (else null). */
export function toolFilePath(name: string, input: string): string | null {
  if (!FILE_PATH_TOOLS.has(name)) return null;
  const obj = parseToolInput(input);
  if (!obj) return null;
  return str(obj.file_path) || str(obj.path) || str(obj.notebook_path) || null;
}

export type EditDiff = { lines: PatchLine[]; additions: number; deletions: number };

/**
 * Line-level diff an Edit/MultiEdit applies, from its `old_string`/`new_string`
 * payload. MultiEdit's edits are stacked, separated by a hunk marker. Returns
 * null for other tools or unparseable/empty input.
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
 * Concise one-line preview of a tool's result (the dimmed `⎿` line under a
 * collapsed row). Strips Read's `N⇥` line-number prefix and skips blank leading
 * lines so the preview is the first meaningful line.
 */
export function resultPreview(content: string, isError: boolean): string {
  const firstMeaningful = content.split("\n").find((line) => line.trim() !== "");
  if (firstMeaningful === undefined) return isError ? "(error)" : "(no output)";
  // `cat -n`-style "  12\tcode" → "code"; leave other content untouched.
  const cleaned = firstMeaningful.replace(/^\s*\d+\t/, "").trim();
  return cleaned || (isError ? "(error)" : "(no output)");
}

/** Expanded view of a tool's input: pretty JSON, else the raw string (so a
 *  malformed payload is still inspectable). */
export function prettyToolInput(input: string): string {
  if (!input) return "";
  try {
    return JSON.stringify(JSON.parse(input), null, 2);
  } catch {
    return input;
  }
}

/** The doc kind a `.md` path implies from the directory it sits in, else null. */
export function docKindForPath(path: string): DocKind | null {
  if (!path.endsWith(".md")) return null;
  if (path.includes("/research/")) return "research";
  if (path.includes("/plans/") || path.includes("/plan/")) return "plan";
  if (path.includes("/handoffs/") || path.includes("/handoff/")) return "handoff";
  return null;
}

// Every `.md` path a shell command mentions: bare, single- or double-quoted.
// Mentions only. Which of them the command *wrote* is deliberately not asked
// here — see `docFromTool` for why the shell can't answer that.
const SH_MD_MENTION = /'([^']+\.md)'|"([^"]+\.md)"|([^\s'"<>|;&()]+\.md)/g;

/**
 * If a tool created a research/plan/handoff `.md` (under `…/research/…`,
 * `…/plans/…`, or `…/handoffs/…`), return its path + kind, to surface
 * agent-created docs in the diff sidebar. Anything else returns null.
 *
 * `Bash` counts because the doc often doesn't arrive through `Write`: the
 * plan/research skills delegate the draft to a subagent (`draughtsman`,
 * `cartographer`) and then persist the returned body with a heredoc, `cp`, or a
 * script that opens the path itself.
 *
 * A shell command is read as a list of *mentions*, not parsed for write
 * position, because the text cannot settle either question that matters. Which
 * path got written needs a shell grammar that a regex loses to (a `2>/dev/null`
 * anywhere makes every other mention look like a target; a heredoc body isn't
 * shell at all; `cp a.md b.md dir/` writes neither operand it names). And what
 * the path resolves to is not in the text at all — `"$OUT"`, `~/…`, and the
 * relative `thoughts/shared/plans/…` the skills are told to use all name a file
 * no consumer can open.
 *
 * `written` settles both: it holds the docs the thoughts index actually saw
 * written, so a mention absent from it is a read (the archivist `grep`s old
 * plans constantly, and must not fill the Documents list with them), and a
 * mention present in it resolves to the canonical absolute path. `Write`/`Edit`
 * need no such proof — `file_path` is unambiguously the file written, and it may
 * legitimately sit outside a thoughts root, where the index never sees it.
 *
 * `written` is required rather than defaulted: a caller that forgot it would
 * silently stop detecting every shell-written doc, which is the bug this
 * function exists to fix.
 */
export function docFromTool(
  name: string,
  input: string,
  written: WrittenDocs,
): { path: string; kind: DocKind } | null {
  const obj = parseToolInput(input);
  if (!obj) return null;
  if (name === "Write" || name === "Edit" || name === "MultiEdit") {
    const path = str(obj.file_path);
    const kind = docKindForPath(path);
    return kind ? { path, kind } : null;
  }
  if (name !== "Bash") return null;
  for (const match of str(obj.command).matchAll(SH_MD_MENTION)) {
    const mention = match[1] ?? match[2] ?? match[3];
    if (!mention || docKindForPath(mention) === null) continue;
    const path = written[baseName(mention)];
    if (!path) continue; // mentioned but never written: a read
    // Classify from the resolved path, not the mention: on the (unlikely)
    // filename collision the indexed doc is the one being described.
    const kind = docKindForPath(path);
    if (kind) return { path, kind };
  }
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
