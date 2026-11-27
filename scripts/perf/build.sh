#!/usr/bin/env bash
# Builds what a perf run needs: the release server, the fixture, and the Tauri app with the probe.
# The app gets its own identifier, so it never touches a real profile or hands off to a running app.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"

cargo build --release -p hyprlayer-server --bin hyprlayer-server
cargo build --release -p hyprlayer-e2e --bin perf_fixture

CONFIG='{"identifier":"ai.brightblock.hyprlayer.perf","build":{"beforeBuildCommand":"pnpm build"},"bundle":{"externalBin":[],"createUpdaterArtifacts":false}}'
VITE_PERF_PROBE=1 pnpm tauri build --no-bundle --features perf-probe --config "$CONFIG"
ls -la "$TARGET/release/Hyprlayer" "$TARGET/release/perf_fixture" "$TARGET/release/hyprlayer-server"
