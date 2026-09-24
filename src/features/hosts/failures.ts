// Codes come from `supervisor::classify_ssh_failure`; anything unrecognised
// still lands here rather than rendering a code at the user.
const FAILURE_SHORT: Record<string, string> = {
  remote_cli_missing: "No server",
  remote_auth_failed: "Auth failed",
  remote_unreachable: "Unreachable",
  remote_platform_unsupported: "Unsupported",
  protocol_version_mismatch: "Version mismatch",
  server_disconnected: "Disconnected",
};

export function shortFailure(code: string | undefined): string {
  return (code && FAILURE_SHORT[code]) || "Failed";
}
