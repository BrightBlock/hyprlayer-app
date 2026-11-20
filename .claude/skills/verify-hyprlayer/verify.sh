#!/usr/bin/env bash
# Launch, health-check and tear down a drivable Hyprlayer dev instance. Usage in SKILL.md.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/../../.." && pwd)"
RUNS="${TMPDIR:-/tmp}/hl-verify"
SUPPORT="$HOME/Library/Application Support"
LOGS="$HOME/Library/Logs"
DEV_ID="ai.brightblock.hyprlayer.dev"

die() { echo "verify: $*" >&2; exit 1; }

# The shell exports the release profile's dirs, and a nested Claude session's vars leak into agents.
CLEAN_ENV=(env -u HYPRLAYER_APP_DATA_DIR -u HYPRLAYER_APP_CONFIG_DIR -u HYPRLAYER_APP_LOG_DIR
  -u HYPRLAYER_AGENT_TOKEN -u HYPRLAYER_SOCKET -u HYPRLAYER_WORKSTREAM_ID
  -u HYPRLAYER_TERMINAL_ID -u HYPRLAYER_SESSION_ID -u CLAUDECODE -u CLAUDE_CODE_ENTRYPOINT)
clean_env() { "${CLEAN_ENV[@]}" "$@"; }

free_port() {
  local p
  for p in $(seq "$1" "$2"); do
    lsof -nP -iTCP:"$p" -sTCP:LISTEN >/dev/null 2>&1 || { echo "$p"; return; }
  done
  die "no free port in $1-$2"
}

listener_pid() { lsof -nP -t -iTCP:"$1" -sTCP:LISTEN 2>/dev/null | head -1; }

descendants() {
  local kids
  kids=$(ps -axo pid=,ppid= | awk -v p="$1" '$2==p {print $1}')
  for k in $kids; do echo "$k"; descendants "$k"; done
}

run_var() { sed -n "s/^$2=//p" "$1/run.env"; }

cmd_launch() {
  local mode="${1:-}"
  [ "$mode" = isolated ] || [ "$mode" = shared ] || die "launch needs isolated|shared"
  local id run conf port identifier
  if [ "$mode" = shared ]; then
    # Someone else's .dev instance owns 1430: attach to it, never start a competitor.
    if [ -n "$(listener_pid 1430)" ]; then
      echo "shared .dev instance already running (vite pid $(listener_pid 1430)); attach on its bridge port, don't launch"
      exit 0
    fi
    identifier="$DEV_ID"; port=1430; conf="$REPO/src-tauri/tauri.dev.conf.json"
  else
    identifier="ai.brightblock.hyprlayer.verify-$(date +%s)"
    port=$(free_port 1431 1449)
  fi
  id="${identifier##*.}"; [ "$mode" = shared ] && id="dev-$(date +%s)"
  mkdir -p "$RUNS/$id"
  # The app stores resolved paths (/private/var/…), so every path a recipe passes must be canonical.
  run="$(cd "$RUNS/$id" && pwd -P)"
  if [ "$mode" = isolated ]; then
    conf="$run/tauri.verify.conf.json"
    cat > "$conf" <<EOF
{
  "productName": "Hyprlayer Verify",
  "identifier": "$identifier",
  "build": {
    "beforeDevCommand": "cargo build -p hyprlayer-server && pnpm exec vite --port $port --strictPort",
    "devUrl": "http://localhost:$port"
  },
  "app": { "withGlobalTauri": true },
  "bundle": { "externalBin": [] }
}
EOF
  fi
  local evidence="$REPO/target/verify/$id"
  mkdir -p "$evidence"
  # exec, with every fd redirected outside the subshell, so a caller's `| tail` isn't held open.
  (cd "$REPO" && exec "${CLEAN_ENV[@]}" nohup pnpm exec tauri dev --config "$conf") > "$run/dev.log" 2>&1 < /dev/null &
  echo $! > "$run/pid"
  mkdir -p "$run/seed/verify-demo" && echo "# demo" > "$run/seed/verify-demo/README.md"
  printf 'MODE=%s\nIDENTIFIER=%s\nVITE_PORT=%s\nPID=%s\nEVIDENCE=%s\nSEED=%s\n' \
    "$mode" "$identifier" "$port" "$(cat "$run/pid")" "$evidence" "$run/seed/verify-demo" > "$run/run.env"
  echo "run dir: $run"
  echo "waiting for the server connection (cold builds take minutes)…"
  local i
  for i in $(seq 1 900); do
    grep -q "connected to this machine" "$run/dev.log" 2>/dev/null && break
    grep -qE "error\[E|ELIFECYCLE|Port [0-9]+ is already in use" "$run/dev.log" 2>/dev/null && { tail -20 "$run/dev.log"; die "launch failed; run: $0 cleanup $run"; }
    kill -0 "$(cat "$run/pid")" 2>/dev/null || { tail -20 "$run/dev.log"; die "tauri dev exited; run: $0 cleanup $run"; }
    sleep 1
  done
  grep -q "connected to this machine" "$run/dev.log" || die "not connected after 15 min; see $run/dev.log"
  cmd_doctor "$run"
}

cmd_doctor() {
  local run="${1:-}" ok=1
  [ -f "$run/run.env" ] || die "doctor needs a run dir from launch"
  local pid port bridge app server identifier
  pid=$(run_var "$run" PID); port=$(run_var "$run" VITE_PORT); identifier=$(run_var "$run" IDENTIFIER)
  bridge=$(sed -n "s/.*MCP Bridge plugin initialized.*on [0-9.]*:\([0-9]*\).*/\1/p" "$run/dev.log" | tail -1)
  app=$(descendants "$pid" | while read -r p; do ps -o pid=,comm= -p "$p"; done | awk '$2 ~ /target\/debug\/Hyprlayer$/ {print $1}' | head -1 || true)
  check() { if eval "$2"; then echo "ok    $1"; else echo "FAIL  $1"; ok=0; fi; }
  check "tauri dev ($pid) alive" "kill -0 $pid 2>/dev/null"
  check "app binary is this repo's target/debug/Hyprlayer (pid ${app:-none})" "[ -n '$app' ]"
  check "vite owns :$port" "[ -n \"\$(listener_pid $port)\" ]"
  check "bridge :${bridge:-?} owned by the app" "[ -n '$bridge' ] && [ \"\$(listener_pid $bridge)\" = '$app' ]"
  check "app connected to its server" "grep -q 'connected to this machine' '$run/dev.log'"
  server=$(pgrep -f "target/debug/hyprlayer-server" | while read -r s; do lsof -a -p "$s" -U 2>/dev/null | grep -q "$identifier/hyprlayer.sock" && echo "$s"; done | head -1 || true)
  check "server (pid ${server:-none}) owns $identifier/hyprlayer.sock" "[ -n '$server' ]"
  echo "build: $(git -C "$REPO" rev-parse --short HEAD)$(git -C "$REPO" diff --quiet || echo '+dirty')  bridge port: ${bridge:-unknown}  evidence: $(run_var "$run" EVIDENCE)"
  [ "$ok" = 1 ] || exit 1
}

cmd_cleanup() {
  local run="${1:-}"
  [ -f "$run/run.env" ] || die "cleanup needs a run dir from launch"
  local pid mode identifier evidence
  pid=$(run_var "$run" PID); mode=$(run_var "$run" MODE); identifier=$(run_var "$run" IDENTIFIER)
  evidence=$(run_var "$run" EVIDENCE)
  local tree; tree="$(descendants "$pid") $pid"
  kill -TERM $tree 2>/dev/null || true
  sleep 3
  kill -KILL $tree 2>/dev/null || true
  if [ "$mode" = isolated ]; then
    # The server outlives the app by design; stop it on this profile only.
    # Set after clean_env's `-u`s, or they're unset and `stop` falls back to the release profile.
    clean_env HYPRLAYER_APP_DATA_DIR="$SUPPORT/$identifier" HYPRLAYER_APP_CONFIG_DIR="$SUPPORT/$identifier" \
      HYPRLAYER_APP_LOG_DIR="$LOGS/$identifier" "$REPO/target/debug/hyprlayer-server" stop 2>/dev/null || true
    case "$identifier" in ai.brightblock.hyprlayer.verify-*) rm -rf "$SUPPORT/$identifier" "$LOGS/$identifier" ;; esac
  fi
  cp "$run/dev.log" "$evidence/dev.log" 2>/dev/null || true
  rm -rf "$run"
  echo "cleaned $identifier; evidence kept at $evidence"
}

case "${1:-}" in
  launch) shift; cmd_launch "$@" ;;
  doctor) shift; cmd_doctor "$@" ;;
  cleanup) shift; cmd_cleanup "$@" ;;
  *) echo "usage: $0 launch isolated|shared | doctor <run-dir> | cleanup <run-dir>" >&2; exit 2 ;;
esac
