/**
 * Pure presentation helpers for agent tool calls (no React/IPC): turn the
 * compact-JSON `input` string from a `toolUse`/`permissionAsk` event into a
 * Claude-Code-style tool name + one-line argument summary.
 */

import type { DocKind } from "@/lib/domain/docs";
import { diffStrings, type PatchLine } from "@/features/workstreams/diff";

import type { TranscriptItem } from "./transcript";
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

/**
 * `meta.description`, falling back to `meta.name`, out of a Workflow
 * script's head. The Workflow tool's `description` parameter is documented
 * as ignored in favour of the script's own `meta` block, so real tool_use
 * records never carry it — this is the only place the label actually lives.
 * The contract requires `meta` to be a pure literal
 * (`export const meta = { name: '...', description: '...', ... }`, no
 * variables, calls, spreads, or interpolation), so a narrow regex over the
 * object's head is sufficient; there is no reason to evaluate the script to
 * read two string fields. The window is bounded to the first 600 characters
 * so a `description` key inside a later `agent()` call's JSON Schema
 * (schemas commonly carry one) can't be mistaken for meta's own — `name`
 * and `description` are always meta's first two fields by convention.
 */
function workflowMetaLabel(script: string): string {
  const head = script.slice(0, 600);
  const field = (key: string): string => {
    const match = head.match(new RegExp(`\\b${key}\\s*:\\s*(['"])((?:(?!\\1).)*)\\1`));
    return match ? match[2] : "";
  };
  return field("description") || field("name");
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
    // "Agent" is the current wire name; "Task" is the pre-rename name an
    // older CLI still emits. Same tool, both kept.
    case "Task":
    case "Agent":
      return str(obj.description) || str(obj.subagent_type);
    // A Workflow call's `description` parameter is documented as ignored in
    // favour of the script's `meta` block, so callers never pass it — every
    // real tool_use carries only `script`, and `str(obj.description)` reads
    // empty for all of them. Pull the label out of the script's own
    // `meta = { name: '...', description: '...' }` head instead; the
    // `default` arm's first-string-value fallback would return the script
    // body verbatim, since `script` is the only string key present.
    case "Workflow":
      return workflowMetaLabel(str(obj.script)) || str(obj.description);
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

// A `.md` mention at the start of a string: bare, single- or double-quoted.
// Anchored (not global) because callers isolate one candidate word first —
// a redirect target, a command operand — and ask only whether that word is a
// doc mention, not scan a whole command for every mention in it.
const MD_WORD = /^(?:'([^']+\.md)'|"([^"]+\.md)"|([^\s'"<>|;&()]+\.md))/;

/** The `.md` mention `text` starts with (after leading whitespace), or null. */
function mdWord(text: string): string | null {
  const m = MD_WORD.exec(text.trimStart());
  return m ? (m[1] ?? m[2] ?? m[3] ?? null) : null;
}

/** Whitespace-separated tokens, keeping a `'...'` or `"..."` span intact so a
 *  quoted path with no internal spaces survives as one token. */
function tokenize(text: string): string[] {
  return text.match(/'[^']*'|"[^"]*"|\S+/g) ?? [];
}

/** A token's command name: the final path segment, quotes stripped, so
 *  `/usr/bin/cp` and `'cp'` both read as `cp`. */
function commandName(token: string): string {
  const bare = token.replace(/^['"]|['"]$/g, "");
  return baseName(bare);
}

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * Delete a heredoc's body (and its terminating delimiter line), keeping the
 * header line that opened it — the header can carry the write itself
 * (`cat > out.md <<'EOF'`), while the body is prose, not shell. Without this a
 * `.md` path only mentioned in a heredoc's body reads as a write target (the
 * `phantom.md` regression, `toolDisplay.test.ts`).
 *
 * Matching only `\w+` missed three delimiters the skills emit: a quoted one
 * takes any characters (`<<'DOC-BODY'`, `<<'EOF.MD'`), and an unquoted one may
 * be backslash-escaped (`<<\EOF`). Their bodies stayed in the command to be
 * parsed as shell. `<<<` opens a here-string, which has no body to strip.
 */
function stripHeredocBodies(command: string): string {
  const START = /(?<!<)<<(-?)\s*(?:'([^'\n]+)'|"([^"\n]+)"|\\?([\w.-]+))/;
  let out = "";
  let rest = command;
  for (;;) {
    const start = START.exec(rest);
    if (!start) return out + rest;
    const dashVariant = start[1] === "-";
    const delim = start[2] ?? start[3] ?? start[4];
    const headerEnd = start.index + start[0].length;
    const nl = rest.indexOf("\n", headerEnd);
    if (nl === -1) return out + rest; // no body follows on this line: nothing to strip
    out += rest.slice(0, nl + 1);
    const body = rest.slice(nl + 1);
    const TERMINATOR = new RegExp(`^${dashVariant ? "\\t*" : ""}${escapeRegExp(delim)}$`, "m");
    const terminator = TERMINATOR.exec(body);
    if (!terminator) return out; // unterminated heredoc: the rest is body, drop it all
    let after = terminator.index + terminator[0].length;
    if (body[after] === "\n") after += 1;
    rest = body.slice(after);
  }
}

// `>` belongs in the lookbehind too: `2>>` blocks on the `2`, and then the bare
// `>` after it matches on its own and hands the append a target.
const REDIRECT = /(?<![0-9&>])>>?/g;

/** `.md` targets of `>`/`>>` redirects in `simple`, skipping fd-numbered
 *  (`2>`) and combined (`&>`) forms — neither is ever a doc write. */
function redirectTargets(simple: string): string[] {
  const out: string[] = [];
  for (const m of simple.matchAll(REDIRECT)) {
    const word = mdWord(simple.slice(m.index + m[0].length));
    if (word) out.push(word);
  }
  return out;
}

// `cp`/`mv`/`install`/`rsync` take a source and a destination; only the last
// operand is ever written to.
const LAST_OPERAND_WRITES = new Set(["cp", "mv", "install", "rsync"]);

/** Targets a single `;`/`&&`/`||`/`|`/newline-delimited command contributes:
 *  redirects, `tee`'s operands, the last operand of a copy/move command, and
 *  an `NAME=<path>` assignment. The assignment counts only for a name in
 *  `scriptVars`, where a heredoc script opens the destination by reading the
 *  variable. Without that guard an incidental `P=plan.md; grep … 2>/dev/null`
 *  counted `P`'s assignment as a write — PR #237's first attempt, replaced
 *  because of it. */
function simpleCommandTargets(simple: string, scriptVars: ReadonlySet<string>): string[] {
  const out = redirectTargets(simple);

  const tokens = tokenize(simple);
  if (tokens.length > 0) {
    const name = commandName(tokens[0]);
    const operands = tokens.slice(1).filter((t) => !t.startsWith("-"));
    if (name === "tee") {
      for (const op of operands) {
        const word = mdWord(op);
        if (word) out.push(word);
      }
    } else if (LAST_OPERAND_WRITES.has(name) && operands.length > 0) {
      const word = mdWord(operands[operands.length - 1]);
      if (word) out.push(word);
    }
  }

  const assign = /^\s*([A-Za-z_]\w*)=(\S+)\s*$/.exec(simple);
  if (assign && scriptVars.has(assign[1])) {
    const word = mdWord(assign[2]);
    if (word) out.push(word);
  }

  return out;
}

// A shell variable reference: `$OUT`, `${OUT}`, `"$OUT"`.
const VAR_REF = /\$\{?([A-Za-z_]\w*)/g;

/** The variable names a heredoc-bearing command hands to its script: the ones
 *  it references outside their own assignment. Empty when the command opens no
 *  heredoc.
 *
 *  `stripped` has the heredoc *bodies* removed, so a reference has to sit on a
 *  header line, as in `python3 - "$OUT" <<'PY'`. That is where the observed
 *  writes put it, and a quoted delimiter leaves nowhere else — such a body
 *  never expands `$OUT`. A `<<` anywhere in the command used to be proof
 *  enough, so a script that carried a heredoc reported every `.md` path it
 *  assigned as authored. The accepted gap: a body-only `$OUT` under an
 *  unquoted `<<PY` goes undetected. */
function heredocScriptVars(stripped: string): Set<string> {
  const out = new Set<string>();
  if (!stripped.includes("<<")) return out;
  for (const m of stripped.matchAll(VAR_REF)) out.add(m[1]);
  return out;
}

/**
 * Every `.md` path a shell command writes *to*, as opposed to only names.
 *
 * PR #237 first tried this alone and had to replace it: a stray `2>/dev/null`
 * unlocked the assigned-path guess and reported a grepped plan as authored.
 * It works here because it is the narrowing half of an AND — `docFromTool`
 * still requires the thoughts index to have seen the file written, so a
 * mis-parsed target that names no real doc resolves to nothing.
 */
function shellWriteTargets(command: string): string[] {
  const stripped = stripHeredocBodies(command);
  const scriptVars = heredocScriptVars(stripped);
  const out: string[] = [];
  for (const simple of stripped.split(/;|&&|\|\||\||\n/)) {
    out.push(...simpleCommandTargets(simple, scriptVars));
  }
  return out;
}

/** Last three path segments, the part a thoughts symlink preserves. */
function docTail(path: string): string {
  return path.split(/[/\\]+/).slice(-3).join("/");
}

/**
 * The indexed path for a doc `Write` reached by a different route, else the
 * path as written.
 *
 * A worktree's `thoughts/shared` is a symlink into the thoughts store, so one
 * doc arrives as `<worktree>/thoughts/shared/plans/x.md` from `Write` and as
 * `<store>/repos/<project>/shared/plans/x.md` from the index — two strings for
 * one file, which the dedup-by-path selectors listed twice.
 *
 * The shared tail makes the substitution safe: the symlink sits at the scope
 * segment, so an alias spells the same `<scope>/<dir>/<file>.md`, while a
 * same-named doc under another scope or outside the store does not. Without
 * that guard this would inherit the Bash arm's filename-collision gap on a path
 * that is otherwise exact.
 */
function canonicalDocPath(path: string, written: WrittenDocs): string {
  const indexed = written[baseName(path)];
  return indexed && docTail(indexed) === docTail(path) ? indexed : path;
}

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
 * Two guards narrow it, and it needs both. `shellWriteTargets` reduces a Bash
 * command to the `.md` paths it writes *to*, so a `cat`/`grep`/`sed` read of a
 * doc never reads as authoring it — but write-position parsing alone
 * false-positives: PR #237's first attempt had to be replaced because a stray
 * `2>/dev/null` unlocked its assigned-path guess and reported a grepped plan as
 * authored. `written` settles what the shell text cannot. It holds the docs the
 * thoughts index saw written, so a mis-parsed target that names no real doc
 * resolves to nothing, and it supplies the canonical absolute path behind
 * `"$OUT"`, `~/…`, and the relative `thoughts/shared/plans/…` spelling the
 * skills are told to use.
 *
 * What gets through both is a false *negative*: a write shape
 * `shellWriteTargets` misses shows up as a missing chip, which costs less than
 * a foreign doc in the sidebar.
 *
 * `Write`/`Edit` need no such proof — `file_path` names the file written, and
 * it may sit outside a thoughts root, where the index never sees it. They still
 * consult `written` to agree with the shell arm on *which* spelling of a doc to
 * report (see `canonicalDocPath`).
 *
 * `written` is required rather than defaulted: a caller that omits it stops
 * detecting shell-written docs, and nothing reports the loss.
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
    return kind ? { path: canonicalDocPath(path, written), kind } : null;
  }
  if (name !== "Bash") return null;
  for (const target of shellWriteTargets(str(obj.command))) {
    const path = written[baseName(target)];
    if (!path) continue; // a write target the index never confirmed: no doc, or it failed
    // Classify from the resolved path, not the target: on the (unlikely)
    // filename collision the indexed doc is the one being described.
    const kind = docKindForPath(path);
    if (kind) return { path, kind };
  }
  return null;
}

/**
 * The newest handoff doc a session's own transcript wrote, skipping ones the
 * user dismissed. Scans the live session's `items` rather than a
 * workstream-wide selector so a previous session's handoff doesn't pop in as
 * soon as a new session of the same workstream opens.
 *
 * Extracted from `SessionChat`'s handoff-attach `useMemo` so the detection
 * logic is testable without rendering the (2000+ line, no-test-harness)
 * component; `SessionChat` calls this from that same `useMemo`, unchanged in
 * behavior.
 */
export function detectHandoffDoc(
  items: TranscriptItem[],
  written: WrittenDocs,
  dismissed: ReadonlySet<string>,
): string | null {
  for (let i = items.length - 1; i >= 0; i--) {
    const item = items[i];
    if (item.kind !== "tool") continue;
    const doc = docFromTool(item.name, item.input, written);
    if (doc?.kind === "handoff" && !dismissed.has(doc.path)) return doc.path;
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
