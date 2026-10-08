#!/usr/bin/env bash
# The linux builder image's server-only entrypoint: /in holds the source tarball,
# /out receives hyprlayer-server alone, /target is the release leg's warm target dir.
set -euo pipefail

TRIPLE="x86_64-unknown-linux-gnu"
SRC_TARBALL="/in/hyprlayer-src.tgz"
export CARGO_TARGET_DIR="/target"

die() { printf 'error: %s\n' "$*" >&2; exit 1; }

[[ -f "$SRC_TARBALL" ]] || die "source tarball not found: $SRC_TARBALL"

# One build at a time in /target: --touch below only orders runs that don't overlap.
exec 9>"$CARGO_TARGET_DIR/.hyprlayer-build.lock"
flock -n 9 || { echo "==> Waiting for another build in /target"; flock 9; }

rm -rf /build
mkdir -p /build
# Other trees build into /target too, and cargo trusts source mtimes.
tar -xzf "$SRC_TARBALL" -C /build --touch
cd /build

# The release leg's own invocation, so both compile the same units.
HYPRLAYER_SIDECAR_TARGET="$TRIPLE" node scripts/build-server-sidecar.mjs
BIN="src-tauri/binaries/hyprlayer-server-$TRIPLE"
[[ -f "$BIN" ]] || die "build-server-sidecar.mjs staged nothing at $BIN"
cp "$BIN" "/out/hyprlayer-server-$TRIPLE"

echo "SERVER_BIN=/out/hyprlayer-server-$TRIPLE"
echo "SERVER_SHA=$(sha256sum "$BIN" | awk '{print $1}')"
echo "SERVER_VERSION=$(sed -nE 's/^version = "(.*)"$/\1/p' crates/hyprlayer-server/Cargo.toml)"
