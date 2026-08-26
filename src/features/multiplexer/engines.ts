
import { useMemo } from "react";

import type { AppSettings, Engine, MultiplexerSettings } from "@/lib/bindings";
import { useBootstrap } from "@/lib/queries";

export type ResolvedEngine = {
  id: string;
  label: string;
  glyph: string;
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

export const engineLabel = (engineId: string): string =>
  BUILTIN_ENGINES.find((e) => e.id === engineId)?.label ?? engineId;

export const enabledEngines = (cfg: ResolvedMultiplexer): ResolvedEngine[] =>
  cfg.engines.filter((e) => e.enabled);

export const engineById = (
  cfg: ResolvedMultiplexer,
  id: string | undefined,
): ResolvedEngine | undefined => (id ? cfg.engines.find((e) => e.id === id) : undefined);

export function engineOrFallback(cfg: ResolvedMultiplexer, engineId: string): ResolvedEngine {
  return engineById(cfg, engineId) ?? resolveEngine({ id: engineId, command: engineId });
}

export function defaultEngine(cfg: ResolvedMultiplexer): ResolvedEngine | undefined {
  const byId = engineById(cfg, cfg.defaultEngineId);
  if (byId?.enabled) return byId;
  return enabledEngines(cfg)[0] ?? cfg.engines[0];
}

export type SessionMode = "assign" | "exec-mint" | "capture" | "none";

type EngineSession = {
  mode: SessionMode;
  fresh: (id?: string) => string[];
  resumeById?: (id: string) => string[];
  resumeLatest: string[];
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
  gemini: {
    mode: "assign",
    fresh: (id) => (id ? ["--session-id", id] : []),
    resumeById: (id) => ["--resume", id],
    resumeLatest: ["--resume", "latest"],
    bypass: ["--yolo"],
  },
  codex: {
    mode: "exec-mint",
    fresh: (id) => (id ? ["resume", id] : []),
    resumeById: (id) => ["resume", id],
    resumeLatest: ["resume", "--last"],
    bypass: ["--dangerously-bypass-approvals-and-sandbox"],
  },
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

export const engineBypassArgs = (engineId: string): string[] => engineSession(engineId).bypass;

export const engineMintsViaExec = (engineId: string): boolean =>
  engineSession(engineId).mode === "exec-mint";

export const engineResumesById = (engineId: string): boolean =>
  engineSession(engineId).resumeById != null;

export const engineChecksSessionOnDisk = (engineId: string): boolean =>
  engineId === "claude-code" || engineId === "codex" || engineId === "antigravity";

export function mintPtySessionId(engineId: string): string | undefined {
  return engineSession(engineId).mode === "assign" ? globalThis.crypto?.randomUUID?.() : undefined;
}

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

export function useMultiplexer(): ResolvedMultiplexer {
  const { data } = useBootstrap();
  return useMemo(() => resolveMultiplexer(data?.settings), [data?.settings]);
}
