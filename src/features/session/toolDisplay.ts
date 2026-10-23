
import type { DocType } from "@/lib/domain/docs";
import { diffStrings, type PatchLine } from "@/lib/diff";

import type { TranscriptItem } from "./transcript";
import type { WrittenDocs } from "./writtenDocs";

const parsedInputCache = new Map<string, Record<string, unknown> | null>();
const PARSED_CACHE_MAX = 20_000;

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

function workflowMetaLabel(script: string): string {
  const head = script.slice(0, 600);
  const field = (key: string): string => {
    const match = head.match(new RegExp(`\\b${key}\\s*:\\s*(['"])((?:(?!\\1).)*)\\1`));
    return match ? match[2] : "";
  };
  return field("description") || field("name");
}

function firstLine(value: string): string {
  const trimmed = value.trim();
  const nl = trimmed.indexOf("\n");
  return nl === -1 ? trimmed : `${trimmed.slice(0, nl).trim()} …`;
}

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
    case "Agent":
      return str(obj.description) || str(obj.subagent_type);
    case "Workflow":
      return workflowMetaLabel(str(obj.script)) || str(obj.description);
    case "Skill": {
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

export function subagentType(name: string, input: string): string {
  if (name !== "Agent" && name !== "Task") return "";
  return str(parseToolInput(input)?.subagent_type);
}

const FILE_PATH_TOOLS = new Set(["Read", "Write", "Edit", "MultiEdit", "NotebookEdit"]);

export function toolFilePath(name: string, input: string): string | null {
  if (!FILE_PATH_TOOLS.has(name)) return null;
  const obj = parseToolInput(input);
  if (!obj) return null;
  return str(obj.file_path) || str(obj.path) || str(obj.notebook_path) || null;
}

export type EditDiff = { lines: PatchLine[]; additions: number; deletions: number };

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

export function baseName(path: string): string {
  const trimmed = path.replace(/[/\\]+$/, "");
  const cut = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  return trimmed.slice(cut + 1) || trimmed;
}

export function resultPreview(content: string, isError: boolean): string {
  const firstMeaningful = content.split("\n").find((line) => line.trim() !== "");
  if (firstMeaningful === undefined) return isError ? "(error)" : "(no output)";
  const cleaned = firstMeaningful.replace(/^\s*\d+\t/, "").trim();
  return cleaned || (isError ? "(error)" : "(no output)");
}

export function prettyToolInput(input: string): string {
  if (!input) return "";
  try {
    return JSON.stringify(JSON.parse(input), null, 2);
  } catch {
    return input;
  }
}

export function docKindForPath(path: string): DocType | null {
  if (!path.endsWith(".md")) return null;
  if (path.includes("/research/")) return "research";
  if (path.includes("/plans/") || path.includes("/plan/")) return "plan";
  if (path.includes("/handoffs/") || path.includes("/handoff/")) return "handoff";
  return null;
}

const MD_WORD = /^(?:'([^']+\.md)'|"([^"]+\.md)"|([^\s'"<>|;&()]+\.md))/;

function mdWord(text: string): string | null {
  const m = MD_WORD.exec(text.trimStart());
  return m ? (m[1] ?? m[2] ?? m[3] ?? null) : null;
}

function tokenize(text: string): string[] {
  return text.match(/'[^']*'|"[^"]*"|\S+/g) ?? [];
}

function commandName(token: string): string {
  const bare = token.replace(/^['"]|['"]$/g, "");
  return baseName(bare);
}

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

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
    if (nl === -1) return out + rest;
    out += rest.slice(0, nl + 1);
    const body = rest.slice(nl + 1);
    const TERMINATOR = new RegExp(`^${dashVariant ? "\\t*" : ""}${escapeRegExp(delim)}$`, "m");
    const terminator = TERMINATOR.exec(body);
    if (!terminator) return out;
    let after = terminator.index + terminator[0].length;
    if (body[after] === "\n") after += 1;
    rest = body.slice(after);
  }
}

const REDIRECT = /(?<![0-9&>])>>?/g;

function redirectTargets(simple: string): string[] {
  const out: string[] = [];
  for (const m of simple.matchAll(REDIRECT)) {
    const word = mdWord(simple.slice(m.index + m[0].length));
    if (word) out.push(word);
  }
  return out;
}

const LAST_OPERAND_WRITES = new Set(["cp", "mv", "install", "rsync"]);

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

const VAR_REF = /\$\{?([A-Za-z_]\w*)/g;

function heredocScriptVars(stripped: string): Set<string> {
  const out = new Set<string>();
  if (!stripped.includes("<<")) return out;
  for (const m of stripped.matchAll(VAR_REF)) out.add(m[1]);
  return out;
}

const writeTargetsCache = new Map<string, string[]>();

// Transcript scans rerun on every streamed delta; a command's targets never change.
function shellWriteTargets(command: string): string[] {
  const cached = writeTargetsCache.get(command);
  if (cached) return cached;
  const stripped = stripHeredocBodies(command);
  const scriptVars = heredocScriptVars(stripped);
  const out: string[] = [];
  for (const simple of stripped.split(/;|&&|\|\||\||\n/)) {
    out.push(...simpleCommandTargets(simple, scriptVars));
  }
  if (writeTargetsCache.size >= PARSED_CACHE_MAX) {
    const oldest = writeTargetsCache.keys().next().value;
    if (oldest !== undefined) writeTargetsCache.delete(oldest);
  }
  writeTargetsCache.set(command, out);
  return out;
}

function docTail(path: string): string {
  return path.split(/[/\\]+/).slice(-3).join("/");
}

// Maps a path written through a worktree's thoughts symlink onto the index's canonical path.
export function canonicalDocPath(path: string, written: WrittenDocs): string {
  const indexed = written[baseName(path)];
  return indexed && docTail(indexed) === docTail(path) ? indexed : path;
}

export function docFromTool(
  name: string,
  input: string,
  written: WrittenDocs,
): { path: string; kind: DocType } | null {
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
    if (!path) continue;
    const kind = docKindForPath(path);
    if (kind) return { path, kind };
  }
  return null;
}

type SessionDocs = { written: WrittenDocs; docs: { path: string; kind: DocType }[] };

const sessionDocsCache = new WeakMap<TranscriptItem[], SessionDocs>();

export function sessionDocs(
  items: TranscriptItem[],
  written: WrittenDocs,
): { path: string; kind: DocType }[] {
  const hit = sessionDocsCache.get(items);
  if (hit && hit.written === written) return hit.docs;
  const docs: { path: string; kind: DocType }[] = [];
  for (const item of items) {
    if (item.kind !== "tool") continue;
    const doc = docFromTool(item.name, item.input, written);
    if (doc) docs.push(doc);
  }
  sessionDocsCache.set(items, { written, docs });
  return docs;
}

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

