//! Frontend view of the multiplexer engine config.
//!
//! The generated `MultiplexerSettings`/`Engine` bindings are all-optional (serde
//! defaults), so this module resolves them into fully-populated shapes and
//! exposes the small queries the UI needs: which engines show in the `+` menu,
//! and which one ⌘T launches. Built-ins mirror the backend seed in settings.rs.

import { useMemo } from "react";

import type { AppSettings, Engine, MultiplexerSettings } from "@/lib/bindings";
import { useBootstrap } from "@/lib/queries";

export type ResolvedEngine = {
  id: string;
  label: string;
  glyph: string;
  /** A theme custom-property name, applied as `var(<token>)`, never raw hex. */
  accentToken: string;
  command: string;
  args: string[];
  enabled: boolean;
  builtin: boolean;
};

export type ResolvedMultiplexer = {
  enabled: boolean;
  defaultEngineId: string;
  notifyOnBlocked: boolean;
  persistAcrossRestarts: boolean;
  engines: ResolvedEngine[];
};

export const DEFAULT_ENGINE_ID = "claude-code";

/** Built-ins, mirroring `Engine::builtins()` in settings.rs. Used when a
 *  settings object predates multiplexer mode (engines undefined). */
export const BUILTIN_ENGINES: ResolvedEngine[] = [
  { id: "claude-code", label: "Claude Code", glyph: "✻", accentToken: "--color-accent", command: "claude", args: [], enabled: true, builtin: true },
  { id: "codex", label: "Codex", glyph: "◆", accentToken: "--color-validate", command: "codex", args: [], enabled: true, builtin: true },
  { id: "gemini", label: "Gemini", glyph: "✦", accentToken: "--color-brand", command: "gemini", args: [], enabled: true, builtin: true },
  { id: "antigravity", label: "Antigravity", glyph: "◉", accentToken: "--color-commit", command: "agy", args: [], enabled: true, builtin: true },
];

function resolveEngine(e: Engine): ResolvedEngine {
  return {
    id: e.id ?? "",
    label: e.label || e.id || "Engine",
    glyph: e.glyph || "▸",
    accentToken: e.accentToken || "--color-accent",
    command: e.command ?? "",
    args: e.args ?? [],
    enabled: e.enabled ?? true,
    builtin: e.builtin ?? false,
  };
}

export function resolveMultiplexer(settings?: AppSettings | null): ResolvedMultiplexer {
  const m: MultiplexerSettings = settings?.multiplexer ?? {};
  const resolved =
    m.engines && m.engines.length > 0
      ? m.engines.map(resolveEngine).filter((e) => e.id)
      : BUILTIN_ENGINES;
  // Merge in any built-in the persisted list predates (e.g. a new engine shipped
  // in an app update), so existing users get it without re-seeding. Builtins are
  // only ever disabled, never removed, so a missing builtin id means "new".
  const missing = BUILTIN_ENGINES.filter((b) => !resolved.some((e) => e.id === b.id));
  const engines = [...resolved, ...missing];
  return {
    enabled: m.enabled ?? false,
    defaultEngineId: m.defaultEngineId ?? DEFAULT_ENGINE_ID,
    notifyOnBlocked: m.notifyOnBlocked ?? true,
    persistAcrossRestarts: m.persistAcrossRestarts ?? true,
    engines,
  };
}

/** A display label for an engine id, from the built-ins (custom engines fall back
 *  to the id). Used where the resolved settings aren't on hand, e.g. labelling an
 *  archived PTY agent row. */
export const engineLabel = (engineId: string): string =>
  BUILTIN_ENGINES.find((e) => e.id === engineId)?.label ?? engineId;

export const enabledEngines = (cfg: ResolvedMultiplexer): ResolvedEngine[] =>
  cfg.engines.filter((e) => e.enabled);

export const engineById = (
  cfg: ResolvedMultiplexer,
  id: string | undefined,
): ResolvedEngine | undefined => (id ? cfg.engines.find((e) => e.id === id) : undefined);

/** Resolve an engine by id, falling back to a generic stub when it was disabled
 *  or removed after a tab was created (so the tab still renders + can relaunch). */
export function engineOrFallback(cfg: ResolvedMultiplexer, engineId: string): ResolvedEngine {
  // resolveEngine fills the same defaults (glyph "▸", --color-accent, enabled,
  // non-builtin) from a sparse engine, so the stub stays in one place.
  return engineById(cfg, engineId) ?? resolveEngine({ id: engineId, command: engineId });
}

/** The engine ⌘T / a new tab launches: the configured default if it's enabled,
 *  else the first enabled engine, else the first engine of all. */
export function defaultEngine(cfg: ResolvedMultiplexer): ResolvedEngine | undefined {
  const byId = engineById(cfg, cfg.defaultEngineId);
  if (byId?.enabled) return byId;
  return enabledEngines(cfg)[0] ?? cfg.engines[0];
}

/** How an engine handles conversation identity, so each tab owns a distinct,
 *  resumable conversation instead of all same-dir tabs sharing "most recent":
 *   - "assign": the CLI takes a caller-chosen id, so we mint a UUID per tab and
 *     pin it as `--session-id` (+ `--resume`). Claude & Gemini, verified against
 *     `--help`. (Antigravity's `--project <id>` looks similar but is rejected in
 *     interactive mode, "Project Not Found", so it can't assign; it captures.)
 *   - "exec-mint": the CLI auto-assigns the id and can't take ours, but a
 *     `codex exec --json` probe announces it as its first event, so we mint it
 *     deterministically up front (no disk-attribution race) and launch
 *     `codex resume <id>`. Codex.
 *   - "capture": the CLI auto-assigns the id; launch bare, then read the id back
 *     from its on-disk store and resume by it. Antigravity (`--conversation <id>`).
 *   - "none": custom engines, no per-session resume; most-recent fallback only. */
export type SessionMode = "assign" | "exec-mint" | "capture" | "none";

/** Per-engine session + bypass behaviour for (re)launching. `fresh`/`resumeById`/
 *  `resumeLatest` build the session args; `resumeLatest` is the "most recent in
 *  this cwd" fallback used before an id is known. Kept frontend-side, the
 *  backend `agent_open` just runs command + args. */
type EngineSession = {
  mode: SessionMode;
  /** Args to start a fresh conversation; "assign" pins `id`, others ignore it. */
  fresh: (id?: string) => string[];
  /** Args to resume the conversation `id` (omitted ⇒ engine has no by-id resume). */
  resumeById?: (id: string) => string[];
  /** Most-recent-conversation resume, used when no id is known yet. */
  resumeLatest: string[];
  /** Bypass-permissions (YOLO) flags. Empty ⇒ no shield for this engine. */
  bypass: string[];
};

const ENGINE_SESSIONS: Record<string, EngineSession> = {
  "claude-code": {
    mode: "assign",
    fresh: (id) => (id ? ["--session-id", id] : []),
    resumeById: (id) => ["--resume", id],
    resumeLatest: ["--continue"],
    bypass: ["--dangerously-skip-permissions"],
  },
  // `gemini --help` (v0.49): --session-id pins a chosen UUID, --resume <id>
  // resumes it, --resume latest is most-recent; --yolo auto-approves.
  gemini: {
    mode: "assign",
    fresh: (id) => (id ? ["--session-id", id] : []),
    resumeById: (id) => ["--resume", id],
    resumeLatest: ["--resume", "latest"],
    bypass: ["--yolo"],
  },
  // Codex auto-assigns a UUID (no caller-set id), so we mint it via a
  // `codex exec` probe and always launch `codex resume <id>`, fresh just resumes
  // the freshly-minted (empty) session. `resume` is a subcommand and accepts the
  // bypass flag after the id (`codex resume <id> --dangerously-…`).
  codex: {
    mode: "exec-mint",
    fresh: (id) => (id ? ["resume", id] : []),
    resumeById: (id) => ["resume", id],
    resumeLatest: ["resume", "--last"],
    bypass: ["--dangerously-bypass-approvals-and-sandbox"],
  },
  // Antigravity auto-assigns a conversation id and can't take ours: interactive
  // `agy --project <id>` warns "Project Not Found" and ignores it (only `--print`
  // mode auto-creates a project, which we can't use for the interactive tab). So
  // launch bare, read the conversation id from disk (capture), and resume by it
  // with `--conversation <id>`; most-recent fallback is `--continue`.
  antigravity: {
    mode: "capture",
    fresh: () => [],
    resumeById: (id) => ["--conversation", id],
    resumeLatest: ["--continue"],
    bypass: ["--dangerously-skip-permissions"],
  },
};

const NO_SESSION: EngineSession = { mode: "none", fresh: () => [], resumeLatest: [], bypass: [] };

export function engineSession(engineId: string): EngineSession {
  return ENGINE_SESSIONS[engineId] ?? NO_SESSION;
}

/** The bypass-permissions flags for an engine (empty ⇒ the tab shows no shield). */
export const engineBypassArgs = (engineId: string): string[] => engineSession(engineId).bypass;

/** Whether we mint this engine's id up front via a `codex exec` probe (codex),
 *  rather than handing it one (assign) or reading it back from disk (capture). */
export const engineMintsViaExec = (engineId: string): boolean =>
  engineSession(engineId).mode === "exec-mint";

/** Whether an engine can resume a specific conversation by id, gates the on-disk
 *  capture, which only helps engines we can later resume by id (antigravity, and
 *  codex as a fallback if its exec-mint probe failed). Assign engines also resume
 *  by id but always already have one, so the capture's `!sessionId` guard skips. */
export const engineResumesById = (engineId: string): boolean =>
  engineSession(engineId).resumeById != null;

/** Whether we can authoritatively check this engine's conversation on disk (the
 *  `agent_session_exists` backend), Claude, Codex, and Antigravity store under the
 *  id we hold (Antigravity's captured `--conversation` id maps to its brain dir).
 *  Gemini/custom fall back to the typed-something heuristic for the resume gate. */
export const engineChecksSessionOnDisk = (engineId: string): boolean =>
  engineId === "claude-code" || engineId === "codex" || engineId === "antigravity";

/** A fresh conversation id for a new tab: a minted UUID for "assign" engines
 *  (which pin it at launch), else undefined, "exec-mint" engines mint via a probe
 *  after creation, "capture" engines read theirs from disk, "none" can't resume. */
export function mintPtySessionId(engineId: string): string | undefined {
  return engineSession(engineId).mode === "assign" ? globalThis.crypto?.randomUUID?.() : undefined;
}

/** The full args for (re)launching an engine: base args, plus the session args
 *  (fresh vs resume-by-id vs most-recent), plus bypass flags. Resuming by a known
 *  `sessionId` continues exactly that tab's conversation; without one it falls
 *  back to the engine's most-recent resume. The cold-start relaunch, the
 *  status-bar Restart, and the shield all build args through here. */
export function relaunchArgs(
  engine: ResolvedEngine,
  opts: { sessionId?: string; resume: boolean; bypass: boolean },
): string[] {
  const s = engineSession(engine.id);
  const sessionArgs = opts.resume
    ? opts.sessionId && s.resumeById
      ? s.resumeById(opts.sessionId)
      : s.resumeLatest
    : s.fresh(opts.sessionId);
  return [...engine.args, ...sessionArgs, ...(opts.bypass ? s.bypass : [])];
}

/** Reactive multiplexer config from the bootstrap query. */
export function useMultiplexer(): ResolvedMultiplexer {
  const { data } = useBootstrap();
  return useMemo(() => resolveMultiplexer(data?.settings), [data?.settings]);
}
