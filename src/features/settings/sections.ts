export const SETTINGS_SECTIONS = [
  "general",
  "harnesses",
  "git",
  "notifications",
  "goals",
  "keyboard",
  "appearance",
  "servers",
  "advanced",
  "binaries",
] as const;

export type SettingsSection = (typeof SETTINGS_SECTIONS)[number];

export function parseSettingsSection(raw: unknown): SettingsSection {
  return (SETTINGS_SECTIONS as readonly unknown[]).includes(raw)
    ? (raw as SettingsSection)
    : "general";
}
