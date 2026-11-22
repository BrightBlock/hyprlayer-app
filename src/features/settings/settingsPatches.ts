import type {
  AgentControlSettings,
  AppSettings,
  GoalSettings,
  GuardrailRule,
  LeadSettings,
} from "@/lib/bindings";

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

export function agentControlSettings(s: AppSettings) {
  const a = s.agentControl ?? {};
  return {
    level: a.level ?? "read",
    maxDepth: a.maxDepth ?? 2,
    maxLiveChildren: a.maxLiveChildren ?? 4,
    maxAgentHops: a.maxAgentHops ?? 6,
    lead: a.lead,
  };
}

export function withAgentControl(s: AppSettings, next: Partial<AgentControlSettings>): AppSettings {
  return { ...s, agentControl: { ...agentControlSettings(s), ...next } };
}

export type Lead = Required<Omit<LeadSettings, "rules">> & { rules: Required<GuardrailRule>[] };

// `null` when the server predates the Lead: it would drop any change to it.
export function leadSettings(s: AppSettings): Lead | null {
  const l = s.agentControl?.lead;
  if (!l) return null;
  return {
    enabled: l.enabled ?? true,
    model: l.model ?? null,
    effort: l.effort ?? null,
    maxLiveChildren: l.maxLiveChildren ?? 6,
    rules: (l.rules ?? []).map((r) => ({
      id: r.id ?? "",
      title: r.title ?? "",
      text: r.text ?? "",
      enabled: r.enabled ?? true,
    })),
    projects: l.projects ?? {},
  };
}

export function withLead(s: AppSettings, next: Partial<Lead>): AppSettings {
  const lead = leadSettings(s);
  return lead ? withAgentControl(s, { lead: { ...lead, ...next } }) : s;
}
