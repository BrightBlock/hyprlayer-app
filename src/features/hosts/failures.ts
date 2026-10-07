// Codes come from `supervisor::classify_ssh_failure`; anything unrecognised
// still lands here rather than rendering a code at the user.
const FAILURE_SHORT: Record<string, string> = {
  remote_cli_missing: "No server",
  remote_auth_failed: "Auth failed",
  remote_sign_in_required: "Sign-in needed",
  remote_sign_in_cancelled: "Sign-in cancelled",
  remote_host_key_changed: "Host key changed",
  remote_unreachable: "Unreachable",
  remote_platform_unsupported: "Unsupported",
  protocol_version_mismatch: "Version mismatch",
  server_disconnected: "Disconnected",
};

export function shortFailure(code: string | undefined): string {
  return (code && FAILURE_SHORT[code]) || "Failed";
}

// Its way out is a command the user runs, so a row shows its remedy rather than a tooltip.
export const HOST_KEY_CHANGED = "remote_host_key_changed";

// Failures another attempt can get past by asking the user, and its button.
const SIGN_IN_ACTION: Record<string, string> = {
  remote_sign_in_required: "Sign in",
  remote_sign_in_cancelled: "Sign in",
  // Asks again when ssh can; "too many authentication failures" never can.
  remote_auth_failed: "Try again",
  remote_host_key_changed: "Try again",
};

export function signInAction(code: string | undefined): string | null {
  return (code && SIGN_IN_ACTION[code]) || null;
}
