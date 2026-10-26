
import type { Harness } from "@/lib/bindings";
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

// Claude Code's own limits for the collapsed Bash header (P3s in 2.1.300).
const COMMAND_MAX_LINES = 2;
const COMMAND_MAX_CHARS = 160;

function commandSummary(value: string): string {
  const command = value.trim();
  const clipped = command
    .split("\n")
    .slice(0, COMMAND_MAX_LINES)
    .join("\n")
    .slice(0, COMMAND_MAX_CHARS);
  return clipped.length < command.length ? `${clipped.trim()}…` : command;
}

// Claude Code lists every arg for MCP and unknown tools; the cap is ours.
const ARGS_MAX_CHARS = 160;

function argsSummary(obj: Record<string, unknown>): string {
  const text = Object.entries(obj)
    .map(([key, value]) => `${key}: ${JSON.stringify(value)}`)
    .join(", ");
  return text.length > ARGS_MAX_CHARS ? `${text.slice(0, ARGS_MAX_CHARS).trim()}…` : text;
}

export type ToolNaming = { harness?: Harness | null; mcpServers: readonly string[]; cwd?: string | null };

// claude's own server-name normalization for `mcp__<server>__<tool>` ids.
function mcpIdSegment(server: string): string {
  const id = server.replace(/[^a-zA-Z0-9_-]/g, "_");
  return server.startsWith("claude.ai ") ? id.replace(/_+/g, "_").replace(/^_|_$/g, "") : id;
}

function mcpParts(name: string, naming?: ToolNaming): { server: string; tool: string } | null {
  const [prefix, server, ...rest] = name.split("__");
  const tool = rest.join("__");
  if (prefix !== "mcp" || !server || !tool) return null;
  if (naming?.harness === "codex") return { server, tool };
  return { server: naming?.mcpServers.find((s) => mcpIdSegment(s) === server) ?? server, tool };
}

// Claude Code 2.1.300's userFacingName for its built-in tools.
const DISPLAY_NAMES: Record<string, string> = {
  Grep: "Search",
  Glob: "Search",
  Edit: "Update",
  MultiEdit: "Update",
  NotebookEdit: "Edit Notebook",
  WebFetch: "Fetch",
  WebSearch: "Web Search",
  TaskStop: "Stop Task",
};

/** A tool as Claude Code's transcript names it; an MCP tool as its own harness writes it. */
export function toolDisplayName(name: string, naming?: ToolNaming): string {
  const mcp = mcpParts(name, naming);
  if (!mcp) return DISPLAY_NAMES[name] ?? name;
  return naming?.harness === "codex" ? `${mcp.server}.${mcp.tool}` : `${mcp.server} - ${mcp.tool} (MCP)`;
}

const BOOKKEEPING_TOOLS = new Set([
  "ToolSearch",
  "ScheduleWakeup",
  "TodoWrite",
  "TaskCreate",
  "TaskGet",
  "TaskUpdate",
  "TaskList",
]);

/** Calls Claude Code leaves out of its transcript unless they fail. */
export function isHiddenTool(item: { name: string; result: { isError: boolean } | null }): boolean {
  return BOOKKEEPING_TOOLS.has(item.name) && !item.result?.isError;
}

function relativePath(path: string, cwd?: string | null): string {
  const root = cwd?.replace(/[/\\]+$/, "");
  if (!root || !path.startsWith(root)) return path;
  const rest = path.slice(root.length);
  if (rest === "") return ".";
  return /^[/\\]/.test(rest) ? rest.slice(1) : path;
}

// codex reports a command as one shlex-joined `bash -lc <script>` string.
const SHELL_WRAPPER = /^(?:\S*\/)?(?:ba|z)?sh\s+-l?c\s+/;

function shellWord(text: string): string | null {
  let out = "";
  let i = 0;
  while (i < text.length && !/\s/.test(text[i])) {
    const c = text[i];
    if (c === "'") {
      const end = text.indexOf("'", i + 1);
      if (end < 0) return null;
      out += text.slice(i + 1, end);
      i = end + 1;
    } else if (c === '"') {
      i += 1;
      while (i < text.length && text[i] !== '"') {
        if (text[i] === "\\" && /["\\$`\n]/.test(text[i + 1] ?? "")) i += 1;
        out += text[i++];
      }
      if (i >= text.length) return null;
      i += 1;
    } else if (c === "\\") {
      out += text[i + 1] ?? "";
      i += 2;
    } else {
      out += c;
      i += 1;
    }
  }
  return text.slice(i).trim() ? null : out;
}

function shellCommand(obj: Record<string, unknown>, naming?: ToolNaming): string {
  const command = str(obj.command);
  if (naming?.harness !== "codex") return command;
  const wrapper = SHELL_WRAPPER.exec(command);
  return (wrapper && shellWord(command.slice(wrapper[0].length))) || command;
}

type Explore = "read" | "search" | "list";

// Claude Code's read-only command classes for collapsing Bash; codex's `sed -n`/`nl`/`cd` reads join them.
const EXPLORE_COMMANDS: Record<string, Explore | "ignore"> = {
  find: "search",
  grep: "search",
  rg: "search",
  ag: "search",
  ack: "search",
  locate: "search",
  which: "search",
  whereis: "search",
  cat: "read",
  head: "read",
  tail: "read",
  less: "read",
  more: "read",
  wc: "read",
  stat: "read",
  file: "read",
  strings: "read",
  jq: "read",
  awk: "read",
  cut: "read",
  sort: "read",
  uniq: "read",
  tr: "read",
  sed: "read",
  nl: "read",
  ls: "list",
  tree: "list",
  du: "list",
  echo: "ignore",
  printf: "ignore",
  true: "ignore",
  false: "ignore",
  ":": "ignore",
  cd: "ignore",
  pwd: "ignore",
};

const EXPLORE_RANK: Record<Explore, number> = { list: 1, read: 2, search: 3 };

const WRITING_FLAGS: Record<string, RegExp> = {
  find: /^-(exec|execdir|ok|okdir|delete|fprint)/,
  sed: /^(-i|--in-place)/,
  sort: /^(-o|--output)/,
};

function shellExploreKind(command: string): Explore | null {
  if (/[`<>]|\$\(/.test(command.replace(/\d?>&\d|\d?>\s*\/dev\/null/g, ""))) return null;
  let kind: Explore | null = null;
  for (const part of command.split(/&&|\|\||[;|\n]/)) {
    const tokens = tokenize(part);
    if (tokens.length === 0) continue;
    const name = commandName(tokens[0]);
    const cls = EXPLORE_COMMANDS[name];
    if (!cls || tokens.some((t) => WRITING_FLAGS[name]?.test(t))) return null;
    if (cls !== "ignore" && (kind === null || EXPLORE_RANK[cls] > EXPLORE_RANK[kind])) kind = cls;
  }
  return kind;
}

export type ExploreKind = { kind: Explore } | { kind: "mcp"; server: string };

/** How a read-only call counts toward a collapsed run, or null when it can't fold. */
export function exploreKind(name: string, input: string, naming?: ToolNaming): ExploreKind | null {
  if (name === "Read") return { kind: "read" };
  if (name === "Grep" || name === "Glob") return { kind: "search" };
  if (name === "Bash") {
    const obj = parseToolInput(input);
    if (!obj || obj.run_in_background === true) return null;
    const kind = shellExploreKind(shellCommand(obj, naming));
    return kind && { kind };
  }
  const mcp = mcpParts(name, naming);
  return mcp && { kind: "mcp", server: mcp.server };
}

const count = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** Claude Code's one-line summary of a collapsed run of reads, searches and MCP calls. */
export function exploreLabel(kinds: readonly ExploreKind[], active: boolean): string {
  const tally = { read: 0, search: 0, list: 0 };
  const servers = new Map<string, number>();
  for (const k of kinds) {
    if (k.kind === "mcp") servers.set(k.server, (servers.get(k.server) ?? 0) + 1);
    else tally[k.kind] += 1;
  }
  const parts: string[] = [];
  if (tally.read) parts.push(`${active ? "reading" : "read"} ${count(tally.read, "file", "files")}`);
  if (tally.search) {
    parts.push(`${active ? "searching" : "searched"} for ${count(tally.search, "pattern", "patterns")}`);
  }
  if (tally.list) {
    parts.push(`${active ? "listing" : "listed"} ${count(tally.list, "directory", "directories")}`);
  }
  for (const [server, n] of servers) {
    parts.push(`${active ? "calling" : "called"} ${server}${n > 1 ? ` ${n} times` : ""}`);
  }
  const text = parts.join(", ");
  return `${text.charAt(0).toUpperCase()}${text.slice(1)}${active ? "…" : ""}`;
}

export function toolSummary(name: string, input: string, naming?: ToolNaming): string {
  const obj = parseToolInput(input);
  if (!obj) return "";
  switch (name) {
    case "Bash":
    case "BashOutput":
      return commandSummary(shellCommand(obj, naming));
    case "Read":
    case "Write":
    case "NotebookEdit":
      return str(obj.file_path) || str(obj.path) || str(obj.notebook_path);
    case "Edit":
    case "MultiEdit":
      return str(obj.file_path);
    case "Glob":
      return str(obj.pattern) + (obj.path ? `  (${relativePath(str(obj.path), naming?.cwd)})` : "");
    case "Grep": {
      const pattern = str(obj.pattern);
      const path = relativePath(str(obj.path), naming?.cwd);
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
    default:
      return argsSummary(obj);
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

export type ResultLine = { text: string; more: number };

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

// claude appends pagination and truncation notes in brackets or parens after the matches.
const isListing = (line: string) => line.trim() !== "" && !/^\s*[[(]/.test(line);

/** The `⎿` line under a tool call, summarized the way Claude Code's own transcript does. */
export function resultLine(
  name: string,
  input: string,
  content: string,
  isError: boolean,
): ResultLine {
  const lines = content.split("\n").filter((line) => line.trim() !== "");
  if (!isError) {
    const obj = parseToolInput(input);
    if (name === "Bash" && obj?.run_in_background === true) {
      return { text: "Running in the background", more: 0 };
    }
    if (name === "Glob" && lines.length > 0 && !/^No files found/.test(lines[0])) {
      return { text: `Found ${plural(lines.filter(isListing).length, "file")}`, more: 0 };
    }
    if (name === "Grep") {
      const matches = str(obj?.output_mode) === "content" && !/^No matches found/.test(lines[0] ?? "");
      return {
        text: matches
          ? `Found ${plural(lines.filter(isListing).length, "line")}`
          : resultPreview(content, false),
        more: 0,
      };
    }
  }
  const text = resultPreview(content.replace(/<\/?tool_use_error>/g, ""), isError);
  return { text, more: Math.max(0, lines.length - 1) };
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

