import type { AgentControlSettings, AppSettings, GoalSettings } from "@/lib/bindings";

// The server's own defaults (crates/hyprlayer-core/src/settings.rs) for slices a fresh profile omits.
export function quickSettings(s: AppSettings) {
  return {
    defaultHarness: s.providers?.defaultHarness ?? "claude",
    notify: s.notifications?.enabled ?? true,
    confirmQuit: s.quit?.confirm ?? true,
  };
}

export function withDefaultHarness(s: AppSettings, key: string): AppSettings {
  return { ...s, providers: { ...(s.providers ?? {}), defaultHarness: key } };
}

export function withNotify(s: AppSettings, enabled: boolean): AppSettings {
  return { ...s, notifications: { enabled, sound: s.notifications?.sound ?? true } };
}

export function withConfirmQuit(s: AppSettings, confirm: boolean): AppSettings {
  return { ...s, quit: { ...(s.quit ?? {}), confirm } };
}

export function goalSettings(s: AppSettings): Required<GoalSettings> {
  const g = s.goals ?? {};
  return {
    maxIterations: g.maxIterations ?? 10,
    maxCostUsd: g.maxCostUsd ?? 10,
    maxTokens: g.maxTokens ?? 2_000_000,
    maxActiveSecs: g.maxActiveSecs ?? 3600,
    evaluatorModel: g.evaluatorModel ?? null,
  };
}

export function withGoals(s: AppSettings, next: Partial<GoalSettings>): AppSettings {
  return { ...s, goals: { ...goalSettings(s), ...next } };
}

export function agentControlSettings(s: AppSettings): Required<AgentControlSettings> {
  const a = s.agentControl ?? {};
  return {
    level: a.level ?? "read",
    maxDepth: a.maxDepth ?? 2,
    maxLiveChildren: a.maxLiveChildren ?? 4,
    maxAgentHops: a.maxAgentHops ?? 6,
  };
}

export function withAgentControl(s: AppSettings, next: Partial<AgentControlSettings>): AppSettings {
  return { ...s, agentControl: { ...agentControlSettings(s), ...next } };
}
