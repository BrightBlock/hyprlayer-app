#!/usr/bin/env bash
# Builds gpui-spike for linux x86_64 (arm64 container, amd64 sysroot) and windows x86_64 (cargo-xwin).
# usage: containers.sh <linux|windows> <out-dir>; leaves timing in <out-dir>/<leg>.time
set -euo pipefail
cd "$(dirname "$0")/../.."
LEG="${1:?linux|windows}"; OUT="$(mkdir -p "${2:?out}" && cd "$2" && pwd)"
SRC="$(mktemp -d)"; trap 'rm -rf "$SRC"' EXIT
git archive HEAD | tar -x -C "$SRC"
if [[ "$LEG" == linux ]]; then
  docker build --quiet --platform linux/arm64 -t g1-spike/linux -f scripts/g1-spike/linux.Dockerfile scripts/g1-spike >/dev/null
  IMG=g1-spike/linux
  CMD='cargo build -p gpui-spike -p hyprlayer-server --target x86_64-unknown-linux-gnu'
else
  IMG=hyprlayer-builder/windows
  CMD='cargo xwin build -p gpui-spike --target x86_64-pc-windows-msvc'
fi
START=$(date +%s)
set +e
docker run --rm --init --platform linux/arm64 --label g1-spike=1 \
  -v "$OUT:/out" -v "$SRC:/src" -v "g1-spike-$LEG-target:/target" -v "g1-spike-$LEG-cargo:/usr/local/cargo/registry" \
  -e CARGO_TARGET_DIR=/target -w /src --entrypoint bash "$IMG" -c "$CMD 2>&1; echo EXIT=\$? ; du -sh /target; cp /target/*/debug/gpui-spike* /out/ 2>/dev/null; true" \
  > "$OUT/$LEG.log" 2>&1
echo "$(( $(date +%s) - START ))s" > "$OUT/$LEG.time"
