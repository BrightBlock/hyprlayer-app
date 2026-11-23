import { STAGE_PERMISSION_MODES, type PermissionMode, type Stage } from "@/lib/bindings";

export type PermissionModeOption = {
  key: PermissionMode;
  label: string;
  short: string;
  hint?: string;
  danger?: boolean;
};

export const PERMISSION_MODES: PermissionModeOption[] = [
  { key: "default", label: "Ask each time", short: "Ask" },
  { key: "acceptEdits", label: "Accept edits", short: "Edits" },
  { key: "plan", label: "Plan only", short: "Plan" },
  { key: "auto", label: "Auto", short: "Auto" },
  { key: "dontAsk", label: "Don't ask", short: "No ask" },
  { key: "bypassPermissions", label: "Bypass permissions", short: "Bypass", danger: true },
];

// Null for no pick, including a tab stored with the old "stage" key.
export function permissionFlag(key: string | null | undefined): PermissionMode | null {
  return PERMISSION_MODES.find((p) => p.key === key)?.key ?? null;
}

export function permissionModeOption(mode: PermissionMode): PermissionModeOption {
  return PERMISSION_MODES.find((p) => p.key === mode) ?? PERMISSION_MODES[0];
}

export function stagePermissionMode(stage: Stage): PermissionMode {
  return STAGE_PERMISSION_MODES[stage];
}
