#!/usr/bin/env bash
set -euo pipefail

SRC_TARBALL="${1:-$HOME/hyprlayer-src.tgz}"
BUILD_DIR="${2:-$HOME/hyprlayer-release-build}"
TARGET_DIR="${3:-$HOME/.hyprlayer-linux-target}"

step() { printf '==> %s\n' "$*"; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }

for tool in cargo pnpm node tar makepkg sha256sum; do
  command -v "$tool" >/dev/null || die "missing required tool on the Linux builder: $tool"
done

[[ -f "$SRC_TARBALL" ]] || die "source tarball not found: $SRC_TARBALL"

step "Resetting build dir $BUILD_DIR"
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR"

step "Extracting source tarball"
tar -xzf "$SRC_TARBALL" -C "$BUILD_DIR"

cd "$BUILD_DIR"

step "Disabling updater artifacts (Linux installs update through their package manager)"
CONF="src-tauri/tauri.conf.json"
grep -q '"createUpdaterArtifacts"[[:space:]]*:[[:space:]]*true' "$CONF" \
  || echo "warning: createUpdaterArtifacts:true not found; build will need a signing key and likely fail" >&2
tmp="$(mktemp)"
sed -E 's/"createUpdaterArtifacts"[[:space:]]*:[[:space:]]*true/"createUpdaterArtifacts": false/' \
  "$CONF" > "$tmp" && mv "$tmp" "$CONF"

step "Verifying hyprlayer-models + hyprlayer-core build standalone"
cargo check -p hyprlayer-models -p hyprlayer-core   # specta-less: the app enables it, embedders don't

step "pnpm install (frozen lockfile)"
pnpm install --frozen-lockfile

step "tauri build --bundles deb,rpm"
export CARGO_TARGET_DIR="$TARGET_DIR"
pnpm tauri build --bundles deb,rpm

step "Locating packages"
DEB="$(ls -t "$TARGET_DIR"/release/bundle/deb/*.deb 2>/dev/null | head -n1 || true)"
RPM="$(ls -t "$TARGET_DIR"/release/bundle/rpm/*.rpm 2>/dev/null | head -n1 || true)"
[[ -n "$DEB" ]] || die "no *.deb found under $TARGET_DIR/release/bundle/deb"
[[ -n "$RPM" ]] || die "no *.rpm found under $TARGET_DIR/release/bundle/rpm"

step "Rendering AUR PKGBUILD + .SRCINFO"
VERSION="$(node -p 'require("./package.json").version')"
DEB_SHA="$(sha256sum "$DEB" | awk '{print $1}')"
EXPECTED_DEB="Hyprlayer_${VERSION}_amd64.deb"
[[ "$(basename "$DEB")" == "$EXPECTED_DEB" ]] \
  || die "deb is named $(basename "$DEB") but the PKGBUILD expects $EXPECTED_DEB"

AUR_DIR="$BUILD_DIR/aur-out"
rm -rf "$AUR_DIR"
mkdir -p "$AUR_DIR"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@SHA256@/$DEB_SHA/g" \
  packaging/aur/PKGBUILD.in > "$AUR_DIR/PKGBUILD"
if grep -q '@VERSION@\|@SHA256@' "$AUR_DIR/PKGBUILD"; then
  die "PKGBUILD still has unsubstituted placeholders"
fi
(cd "$AUR_DIR" && makepkg --printsrcinfo > .SRCINFO)
[[ -s "$AUR_DIR/.SRCINFO" ]] || die "makepkg produced an empty .SRCINFO"

echo ""
echo "DEB=$DEB"
echo "RPM=$RPM"
echo "AUR_DIR=$AUR_DIR"
