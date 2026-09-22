
import type { PermissionMode } from "@/lib/bindings";

export type PermissionModeOption = {
  key: string;
  label: string;
  hint?: string;
  flag: PermissionMode | null;
  danger?: boolean;
};

export const PERMISSION_MODES: PermissionModeOption[] = [
  { key: "auto", label: "Auto", flag: null },
  { key: "default", label: "Ask each time", flag: "default" },
  { key: "acceptEdits", label: "Accept edits", flag: "acceptEdits" },
  { key: "plan", label: "Plan only", flag: "plan" },
  { key: "bypassPermissions", label: "Bypass permissions", flag: "bypassPermissions", danger: true },
];

export function permissionModeOption(key: string | undefined): PermissionModeOption {
  return PERMISSION_MODES.find((p) => p.key === key) ?? PERMISSION_MODES[0];
}
