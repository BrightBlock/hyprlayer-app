import type { PermissionMode } from "@/lib/bindings";

export type PermissionModeOption = {
  key: string;
  label: string;
  short: string;
  hint?: string;
  flag: PermissionMode | null;
  danger?: boolean;
};

export const PERMISSION_MODES: PermissionModeOption[] = [
  { key: "stage", label: "Stage default", short: "Stage", flag: null },
  { key: "default", label: "Ask each time", short: "Ask", flag: "default" },
  { key: "acceptEdits", label: "Accept edits", short: "Edits", flag: "acceptEdits" },
  { key: "plan", label: "Plan only", short: "Plan", flag: "plan" },
  { key: "auto", label: "Auto", short: "Auto", flag: "auto" },
  { key: "dontAsk", label: "Don't ask", short: "No ask", flag: "dontAsk" },
  {
    key: "bypassPermissions",
    label: "Bypass permissions",
    short: "Bypass",
    flag: "bypassPermissions",
    danger: true,
  },
];

export function permissionModeOption(key: string | undefined): PermissionModeOption {
  return PERMISSION_MODES.find((p) => p.key === key) ?? PERMISSION_MODES[0];
}
