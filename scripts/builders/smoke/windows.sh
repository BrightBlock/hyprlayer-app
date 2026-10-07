#!/usr/bin/env bash
# Installs a Windows build on a real Windows host, launches it in the logged-on
# session on a fresh profile, uninstalls it, and puts back whatever Hyprlayer
# install and profile were there:
#   scripts/builders/smoke/windows.sh <setup.exe> <hyprlayer-server.exe> [out-dir]
# With --check it only confirms the host is reachable and idle, touching nothing.
set -euo pipefail

cd "$(dirname "$0")/../../.."

USAGE="usage: windows.sh <setup.exe> <hyprlayer-server.exe> [out-dir], or windows.sh --check"
HOST="${HYPRLAYER_WIN_TEST_HOST:-nexus-win}"
# cmd.exe is the guest's sshd shell, so %VAR% expands remotely.
REMOTE='%LOCALAPPDATA%\Temp\hl-smoke'
REMOTE_SCP='AppData/Local/Temp/hl-smoke'

say() { printf '\033[1;36m→ %s\033[0m\n' "$*"; }
die() { printf '\033[1;31m✗ %s\033[0m\n' "$*" >&2; exit 1; }
reachable() {
  ssh -o BatchMode=yes -o ConnectTimeout=10 "$HOST" "echo ok" >/dev/null 2>&1 \
    || die "$HOST unreachable over SSH — start it, or set HYPRLAYER_WIN_TEST_HOST"
}

if [[ "${1:-}" == "--check" ]]; then
  reachable
  rc=0
  ssh -o BatchMode=yes "$HOST" 'tasklist /NH | findstr /I /B /L "Hyprlayer.exe hyprlayer-server.exe"' >/dev/null 2>&1 || rc=$?
  case "$rc" in
    1) exit 0 ;;
    0) die "Hyprlayer or its server is running on $HOST: quit both there first" ;;
    *) die "could not list processes on $HOST (ssh exit $rc)" ;;
  esac
fi

SETUP="${1:?$USAGE}"
SERVER="${2:?$USAGE}"
OUT="${3:-target/smoke/windows}"
[[ -f "$SETUP" && -f "$SERVER" ]] || die "$USAGE"
reachable

mkdir -p "$OUT"
REPORT="$OUT/report.txt"
: >"$REPORT"
phase() {
  say "$1"
  ssh -o BatchMode=yes "$HOST" \
    "powershell -NoProfile -ExecutionPolicy Bypass -File $REMOTE\\windows-host.ps1 -Phase $*" 2>&1 | tee -a "$REPORT"
  return "${PIPESTATUS[0]}"
}

ssh -o BatchMode=yes "$HOST" "if not exist $REMOTE mkdir $REMOTE"
scp -q scripts/builders/smoke/windows-host.ps1 "$HOST:$REMOTE_SCP/windows-host.ps1"
scp -q "$SETUP" "$HOST:$REMOTE_SCP/setup.exe"

# Until backup succeeds only the uploads are ours; a backup a failed run left behind is not.
trap 'ssh -o BatchMode=yes "$HOST" "del /q $REMOTE\\setup.exe $REMOTE\\windows-host.ps1" >/dev/null 2>&1 || true' EXIT
phase backup
restore() {
  # The backup holds the only copy of the previous install and profile, so it outlives a failed restore.
  if phase restore; then
    phase cleanup || true
    ssh -o BatchMode=yes "$HOST" "rmdir /s /q $REMOTE" || true
  else
    printf '\033[1;31m✗ restore failed on %s — the previous install and profile are in %s: move profile-roaming back to %s and profile-local to %s\033[0m\n' \
      "$HOST" '%LOCALAPPDATA%\hl-smoke-backup' '%APPDATA%\ai.brightblock.hyprlayer' '%LOCALAPPDATA%\ai.brightblock.hyprlayer' >&2
  fi
}
trap restore EXIT

failed=0
phase install -Setup "$REMOTE\\setup.exe" -ServerSha "$(shasum -a 256 "$SERVER" | awk '{print $1}')" || failed=1
phase launch || failed=1
scp -q "$HOST:$REMOTE_SCP/probe/screenshot.png" "$OUT/screenshot.png" 2>/dev/null || true
phase close || failed=1
phase uninstall || failed=1

[[ "$failed" == "0" ]] || die "smoke test failed on $HOST — see $REPORT"
printf '\033[1;32m✓ %s installs, launches, and uninstalls on %s — %s\033[0m\n' "$(basename "$SETUP")" "$HOST" "$OUT"
