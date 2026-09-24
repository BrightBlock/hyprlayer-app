import type { AppSettings } from "@/lib/bindings";

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
