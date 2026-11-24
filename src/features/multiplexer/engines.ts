
import { useMemo } from "react";

import type { AppSettings, Engine, MultiplexerSettings } from "@/lib/bindings";
import type { StageKey } from "@/lib/domain/stages";
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

type ResolvedMultiplexer = {
  enabled: boolean;
  defaultEngineId: string;
  notifyOnBlocked: boolean;
  persistAcrossRestarts: boolean;
  actionsInCli: boolean;
  engines: ResolvedEngine[];
};

export const DEFAULT_ENGINE_ID = "claude-code";

const BUILTIN_ENGINES: ResolvedEngine[] = [
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
    actionsInCli: m.actionsInCli ?? true,
    engines,
  };
}

export const engineLabel = (engineId: string): string =>
  BUILTIN_ENGINES.find((e) => e.id === engineId)?.label ?? engineId;

export const enabledEngines = (cfg: ResolvedMultiplexer): ResolvedEngine[] =>
  cfg.engines.filter((e) => e.enabled);

const engineById = (
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

/** Where app actions, and a workstream's first agent, open; `undefined` is a native chat. */
export function actionEngine(cfg: ResolvedMultiplexer, stage: StageKey = "general"): ResolvedEngine | undefined {
  const engine = cfg.enabled && cfg.actionsInCli ? defaultEngine(cfg) : undefined;
  // Only claude's launch applies a stage's guardrails; codex's sandbox posture is a native session's.
  if (stage !== "general" && engine?.id !== DEFAULT_ENGINE_ID) return undefined;
  return engine;
}

type SessionMode = "assign" | "exec-mint" | "capture" | "none";

type EngineSession = {
  mode: SessionMode;
  bypass: string[];
};

const ENGINE_SESSIONS: Record<string, EngineSession> = {
  "claude-code": {
    mode: "assign",
    bypass: ["--dangerously-skip-permissions"],
  },
  gemini: {
    mode: "assign",
    bypass: ["--yolo"],
  },
  codex: {
    mode: "exec-mint",
    bypass: ["--dangerously-bypass-approvals-and-sandbox"],
  },
  antigravity: {
    mode: "capture",
    bypass: ["--dangerously-skip-permissions"],
  },
};

const NO_SESSION: EngineSession = { mode: "none", bypass: [] };

function engineSession(engineId: string): EngineSession {
  return Object.prototype.hasOwnProperty.call(ENGINE_SESSIONS, engineId) ? ENGINE_SESSIONS[engineId] : NO_SESSION;
}

export const engineBypassArgs = (engineId: string): string[] => engineSession(engineId).bypass;

export const engineMintsViaExec = (engineId: string): boolean =>
  engineSession(engineId).mode === "exec-mint";

export const engineResumesById = (engineId: string): boolean => engineSession(engineId) !== NO_SESSION;

export const engineChecksSessionOnDisk = (engineId: string): boolean =>
  engineId === "claude-code" || engineId === "codex" || engineId === "antigravity";

export function useMultiplexer(): ResolvedMultiplexer {
  const { data } = useBootstrap();
  return useMemo(() => resolveMultiplexer(data?.settings), [data?.settings]);
}
