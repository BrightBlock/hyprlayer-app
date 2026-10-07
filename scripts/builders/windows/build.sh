#!/usr/bin/env bash
# Runs inside the windows builder image: /in holds the source tarball, /out
# receives the installer and the server, /target is the warm cargo target dir.
set -euo pipefail

TRIPLE="x86_64-pc-windows-msvc"
SRC_TARBALL="/in/hyprlayer-src.tgz"
BUILD_DIR="/build"
OUT="/out"
export CARGO_TARGET_DIR="/target"

step() { printf '==> %s\n' "$*"; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }

[[ -f "$SRC_TARBALL" ]] || die "source tarball not found: $SRC_TARBALL"

step "Extracting source tarball"
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR"
tar -xzf "$SRC_TARBALL" -C "$BUILD_DIR"
cd "$BUILD_DIR"

step "Disabling updater artifacts (signing happens on the Mac)"
CONF="src-tauri/tauri.conf.json"
grep -q '"createUpdaterArtifacts"[[:space:]]*:[[:space:]]*true' "$CONF" \
  || echo "warning: createUpdaterArtifacts:true not found; build will need a signing key and likely fail" >&2
sed -i -E 's/"createUpdaterArtifacts"[[:space:]]*:[[:space:]]*true/"createUpdaterArtifacts": false/' "$CONF"

# Points plain cargo, and the cc crate under it, at clang-cl, lld-link and the
# baked SDK, so the sidecar build in beforeBuildCommand links too.
eval "$(cargo xwin env --target "$TRIPLE")"

step "Verifying hyprlayer-models + hyprlayer-core for $TRIPLE"
cargo check -p hyprlayer-models -p hyprlayer-core --target "$TRIPLE"

step "pnpm install (frozen lockfile)"
pnpm install --frozen-lockfile --store-dir /pnpm-store

step "tauri build --target $TRIPLE --bundles nsis"
HYPRLAYER_SIDECAR_TARGET="$TRIPLE" pnpm tauri build --target "$TRIPLE" --bundles nsis

VERSION="$(node -p 'require("./package.json").version')"
SETUP="$CARGO_TARGET_DIR/$TRIPLE/release/bundle/nsis/Hyprlayer_${VERSION}_x64-setup.exe"
[[ -f "$SETUP" ]] || die "no installer at $SETUP"
SERVER="src-tauri/binaries/hyprlayer-server-$TRIPLE.exe"
[[ -f "$SERVER" ]] || die "no hyprlayer-server sidecar at $SERVER — did beforeBuildCommand run?"

cp "$SETUP" "$SERVER" "$OUT/"
echo ""
echo "SETUP_EXE=$OUT/$(basename "$SETUP")"
echo "SERVER_EXE=$OUT/$(basename "$SERVER")"
