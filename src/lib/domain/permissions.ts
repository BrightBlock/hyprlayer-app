/**
 * The permission-mode choices offered in the composer. These mirror the modes
 * the `claude` CLI exposes (its `--permission-mode` flag / in-prompt picker) and
 * map straight to the driver's `--permission-mode` at spawn time, see
 * `SpawnConfig` (crates/hyprlayer-core/src/claude/driver.rs).
 *
 * Like model/effort, the choice is fixed when the session spawns: changing it
 * only affects the NEXT session you start in this tab. The "auto" option leaves
 * the posture to the stage's own policy (the default behavior).
 */

import type { PermissionMode } from "@/lib/bindings";

/** A selectable permission mode. `flag` is the `--permission-mode` value
 *  (typed by the bindings union, generated from core's `PermissionMode`), or
 *  null for "auto", let the backend stage policy choose. */
export type PermissionModeOption = {
  key: string;
  label: string;
  /** A short qualifier shown after the label. */
  hint?: string;
  flag: PermissionMode | null;
  /** Flags the most permissive mode so the composer can warn on it. */
  danger?: boolean;
};

export const PERMISSION_MODES: PermissionModeOption[] = [
  // Use the stage's default posture (what every session did before).
  { key: "auto", label: "Auto", flag: null },
  { key: "default", label: "Ask each time", flag: "default" },
  { key: "acceptEdits", label: "Accept edits", flag: "acceptEdits" },
  { key: "plan", label: "Plan only", flag: "plan" },
  { key: "bypassPermissions", label: "Bypass permissions", flag: "bypassPermissions", danger: true },
];

export const DEFAULT_PERMISSION_MODE_KEY = "auto";

/** Resolve a stored permission-mode key to its option; an unknown/empty value
 *  falls back to "auto" (the stage default). */
export function permissionModeOption(key: string | undefined): PermissionModeOption {
  return PERMISSION_MODES.find((p) => p.key === key) ?? PERMISSION_MODES[0];
}
