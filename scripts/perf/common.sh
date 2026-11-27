#!/usr/bin/env bash
# Shared by run-linux.sh and run-macos.sh. Sourced, never run.
# One run: fresh fixture and profile, a cold launch with the whole scenario, then a warm relaunch.

set -euo pipefail

PERF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$PERF_DIR/../.." && pwd)"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"
OUT="${PERF_OUT:-$TARGET/perf}/${PERF_CLIENT:-tauri}"
SERVER_BIN="${HYPRLAYER_E2E_SERVER_BIN:-$TARGET/release/hyprlayer-server}"
FIXTURE_BIN="${PERF_FIXTURE_BIN:-$TARGET/release/perf_fixture}"
APP_BIN="${PERF_APP:-$TARGET/release/Hyprlayer}"
IDENTIFIER="ai.brightblock.hyprlayer.perf"
RUNS="${PERF_RUNS:-7}"
SCENARIO="${PERF_SCENARIO:-$PERF_DIR/scenarios/baseline.json}"
STARTUP_SCENARIO="$PERF_DIR/scenarios/startup.json"

FIXTURE_PID=""
APP_PID=""
WATCHER_PID=""

log() { printf '[perf] %s\n' "$*" >&2; }

now_ns() { python3 -c 'import time; print(time.time_ns())'; }

# The profile dirs the perf identifier uses; wiped so every run's cold launch starts clean.
profile_dirs() {
  if [ "$(uname)" = Darwin ]; then
    printf '%s\n' "$HOME/Library/Application Support/$IDENTIFIER" "$HOME/Library/Logs/$IDENTIFIER" \
      "$HOME/Library/Caches/$IDENTIFIER" "$HOME/Library/WebKit/$IDENTIFIER" \
      "$HOME/Library/HTTPStorages/$IDENTIFIER" "$HOME/Library/Saved Application State/$IDENTIFIER.savedState"
  else
    printf '%s\n' "$HOME/.config/$IDENTIFIER" "$HOME/.local/share/$IDENTIFIER" "$HOME/.cache/$IDENTIFIER"
  fi
}

config_dir() {
  if [ "$(uname)" = Darwin ]; then echo "$HOME/Library/Application Support/$IDENTIFIER"; else echo "$HOME/.config/$IDENTIFIER"; fi
}

reset_profile() {
  local d
  while IFS= read -r d; do rm -rf "$d"; done < <(profile_dirs)
  mkdir -p "$(config_dir)"
  printf '{"hosts":[],"active":null,"localProfile":"%s"}\n' "$1" >"$(config_dir)/hosts.json"
}

# The harness library writes beside `<exe>/../../..`, so the fixture runs from a copy whose
# ancestors keep that under target/perf rather than the repo root.
fixture_copy() {
  local copy="$TARGET/perf/fixture/deps/perf_fixture"
  mkdir -p "$(dirname "$copy")"
  cp "$FIXTURE_BIN" "$copy"
  echo "$copy"
}

start_fixture() {
  local dir="$1"
  mkdir -p "$dir"
  HYPRLAYER_E2E_SERVER_BIN="$SERVER_BIN" "$(fixture_copy)" >"$dir/fixture.json" 2>"$dir/fixture.err" &
  FIXTURE_PID=$!
  for _ in $(seq 1 1200); do
    if grep -q '^{' "$dir/fixture.json" 2>/dev/null; then return 0; fi
    kill -0 "$FIXTURE_PID" 2>/dev/null || { log "fixture exited early"; cat "$dir/fixture.err" >&2; return 1; }
    sleep 0.5
  done
  log "fixture never printed its report"
  return 1
}

fixture_field() { node -e 'const j=JSON.parse(require("fs").readFileSync(process.argv[1],"utf8").split("\n").find(l=>l.startsWith("{")));const v=j[process.argv[2]];console.log(Array.isArray(v)?v.join(","):v)' "$1" "$2"; }

stop_fixture() {
  [ -n "$FIXTURE_PID" ] || return 0
  local sandbox
  sandbox="$(fixture_field "$1/fixture.json" sandbox 2>/dev/null || true)"
  [ -n "$sandbox" ] && touch "$sandbox/stop"
  for _ in $(seq 1 40); do kill -0 "$FIXTURE_PID" 2>/dev/null || break; sleep 0.5; done
  kill -0 "$FIXTURE_PID" 2>/dev/null && kill "$FIXTURE_PID" 2>/dev/null || true
  FIXTURE_PID=""
}

# Samples the app's process set at each phase the client announces, then acks it.
watch_phases() {
  local dir="$1" exclude="$2" f name
  mkdir -p "$dir/samples"
  while :; do
    for f in "$dir"/phase-*; do
      [ -e "$f" ] || continue
      name="${f##*/phase-}"
      [ -e "$dir/ack-$name" ] && continue
      node "$PERF_DIR/sample.mjs" --root "$APP_PID" --exclude "$exclude" >"$dir/samples/$name.json" 2>/dev/null || echo '{}' >"$dir/samples/$name.json"
      touch "$dir/ack-$name"
    done
    sleep 0.2
  done
}

webkit_pids() { pgrep -f 'com.apple.WebKit' 2>/dev/null | tr '\n' ',' || true; }

# $1 run dir, $2 scenario, $3 report path, $4 fixture json
launch_app() {
  local dir="$1" scenario="$2" report="$3" fx="$4"
  rm -rf "$dir"/phase-* "$dir"/ack-* "$report"
  local exclude=""
  [ "$(uname)" = Darwin ] && exclude="$(webkit_pids)"
  # T0 is read in the process that execs the app, so interpreter start-up is not counted.
  HYPRLAYER_PERF_SCENARIO="$scenario" HYPRLAYER_PERF_REPORT="$report" HYPRLAYER_PERF_PHASE_DIR="$dir" \
    HYPRLAYER_PERF_BIGFILE="$(fixture_field "$fx" bigFile)" HYPRLAYER_PERF_TERMINALS="$(fixture_field "$fx" terminals)" \
    python3 -c '
import os, sys, time
os.environ["HYPRLAYER_PERF_T0_NS"] = str(time.time_ns())
os.execv(sys.argv[1], [sys.argv[1]])' "$APP_BIN" >"$dir/app.log" 2>&1 &
  APP_PID=$!
  watch_phases "$dir" "$exclude" &
  WATCHER_PID=$!
  post_launch "$dir"
}

post_launch() { :; }

wait_report() {
  local report="$1" limit="$2"
  for _ in $(seq 1 "$limit"); do
    [ -s "$report" ] && return 0
    kill -0 "$APP_PID" 2>/dev/null || { log "the app exited before reporting"; return 1; }
    sleep 1
  done
  log "no report within ${limit}s; last step: $(cat "$(dirname "$report")/breadcrumb.json" 2>/dev/null || echo none)"
  return 1
}

kill_app() {
  [ -n "$WATCHER_PID" ] && kill "$WATCHER_PID" 2>/dev/null || true
  if [ -n "$APP_PID" ]; then
    kill "$APP_PID" 2>/dev/null || true
    for _ in $(seq 1 20); do kill -0 "$APP_PID" 2>/dev/null || break; sleep 0.25; done
    kill -9 "$APP_PID" 2>/dev/null || true
  fi
  WATCHER_PID=""
  APP_PID=""
}

cleanup() { kill_app; [ -n "${CURRENT_DIR:-}" ] && stop_fixture "$CURRENT_DIR" || true; restore_session; }
restore_session() { :; }
trap cleanup EXIT

# One run: writes $OUT/<n>/report.json with the client's metrics plus the harness's.
one_run() {
  local n="$1"
  local dir="$OUT/$n"
  CURRENT_DIR="$dir"
  rm -rf "$dir"
  mkdir -p "$dir"
  log "run $n: fixture"
  start_fixture "$dir"
  reset_profile "$(fixture_field "$dir/fixture.json" appData)"
  log "run $n: cold launch"
  launch_app "$dir" "$SCENARIO" "$dir/client.json" "$dir/fixture.json"
  wait_report "$dir/client.json" "${PERF_RUN_LIMIT:-2400}"
  kill_app
  log "run $n: warm launch"
  launch_app "$dir" "$STARTUP_SCENARIO" "$dir/warm.json" "$dir/fixture.json"
  wait_report "$dir/warm.json" 300 || true
  kill_app
  node "$PERF_DIR/merge.mjs" "$dir"
  stop_fixture "$dir"
  CURRENT_DIR=""
}

run_all() {
  [ -x "$APP_BIN" ] || { log "no app at $APP_BIN; run scripts/perf/build.sh"; exit 1; }
  [ -x "$FIXTURE_BIN" ] && [ -x "$SERVER_BIN" ] || { log "build the fixture and server first (scripts/perf/build.sh)"; exit 1; }
  mkdir -p "$OUT"
  print_environment >"$OUT/environment.json"
  local n
  for n in $(seq "${PERF_FIRST:-1}" "$RUNS"); do
    one_run "$n"
  done
  node "$PERF_DIR/summarize.mjs" "$(dirname "$OUT")"
}
