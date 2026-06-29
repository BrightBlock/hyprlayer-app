//! Frontend view of the multiplexer engine config.
//!
//! The generated `MultiplexerSettings`/`Engine` bindings are all-optional (serde
//! defaults), so this module resolves them into fully-populated shapes and
//! exposes the small queries the UI needs: which engines show in the `+` menu,
//! and which one ⌘T launches. Built-ins mirror the backend seed in settings.rs.

import { useMemo } from "react";

import type { AppSettings, Engine, MultiplexerSettings } from "../../lib/bindings";
import { useBootstrap } from "../../lib/queries";

export type ResolvedEngine = {
  id: string;
  label: string;
  glyph: string;
  /** A theme custom-property name, applied as `var(<token>)` — never raw hex. */
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

/** Per-engine flags for the status-bar Restart / YOLO actions:
 *   resumeArgs — relaunch continuing the prior session (Restart).
 *   bypassArgs — relaunch skipping permission/approval prompts (the red shield).
 * Best-effort defaults for the built-ins; flags can vary by CLI version, and a
 * custom engine has neither (so its tab shows Restart-only, no shield). Kept
 * frontend-side — the backend `agent_open` just runs command + args. */
const ENGINE_ACTIONS: Record<string, { resumeArgs: string[]; bypassArgs: string[] }> = {
  "claude-code": { resumeArgs: ["--continue"], bypassArgs: ["--dangerously-skip-permissions"] },
  codex: { resumeArgs: [], bypassArgs: ["--dangerously-bypass-approvals-and-sandbox"] },
  gemini: { resumeArgs: [], bypassArgs: ["--yolo"] },
  // Verified against `agy --help`: --continue resumes the most recent
  // conversation; --dangerously-skip-permissions auto-approves all tools.
  antigravity: { resumeArgs: ["--continue"], bypassArgs: ["--dangerously-skip-permissions"] },
};

export function engineActions(engineId: string): { resumeArgs: string[]; bypassArgs: string[] } {
  return ENGINE_ACTIONS[engineId] ?? { resumeArgs: [], bypassArgs: [] };
}

/** The full args for (re)launching an engine: base args, plus resume
 *  (`--continue`) when continuing a prior session, plus the bypass flags in
 *  bypass-permissions mode. The cold-start relaunch and the status-bar Restart /
 *  shield all build args through here, so a restored tab resumes exactly as a
 *  manual Restart would. */
export function relaunchArgs(
  engine: ResolvedEngine,
  opts: { resume: boolean; bypass: boolean },
): string[] {
  const a = engineActions(engine.id);
  return [
    ...engine.args,
    ...(opts.resume ? a.resumeArgs : []),
    ...(opts.bypass ? a.bypassArgs : []),
  ];
}

/** Reactive multiplexer config from the bootstrap query. */
export function useMultiplexer(): ResolvedMultiplexer {
  const { data } = useBootstrap();
  return useMemo(() => resolveMultiplexer(data?.settings), [data?.settings]);
}
