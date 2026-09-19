#!/usr/bin/env bash
set -euo pipefail

SRC_TARBALL="${1:-$HOME/hyprlayer-src.tgz}"
BUILD_DIR="${2:-$HOME/hyprlayer-release-build}"
TARGET_DIR="${3:-$HOME/.hyprlayer-linux-target}"

step() { printf '==> %s\n' "$*"; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }

for tool in cargo pnpm node tar bsdtar makepkg sha256sum; do
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

step "Verifying the deb carries hyprlayer-server beside the app"
# supervisor.rs resolves the server as a sibling of current_exe(), so a sidecar
# staged anywhere but /usr/bin is an app that cannot start once installed.
DEB_PATHS="$(bsdtar -xOf "$DEB" 'data.tar*' | bsdtar -tf - || true)"
if ! printf '%s\n' "$DEB_PATHS" | grep -qE '(^|\./)usr/bin/hyprlayer-server$'; then
  printf '%s\n' "$DEB_PATHS" | grep -E 'hyprlayer' | head -20 >&2 || true
  die "the deb has no usr/bin/hyprlayer-server (paths containing 'hyprlayer' listed above)"
fi

step "Rendering AUR PKGBUILD + .SRCINFO"
VERSION="$(node -p 'require("./package.json").version')"
DEB_SHA="$(sha256sum "$DEB" | awk '{print $1}')"
EXPECTED_DEB="Hyprlayer_${VERSION}_amd64.deb"
[[ "$(basename "$DEB")" == "$EXPECTED_DEB" ]] \
  || die "deb is named $(basename "$DEB") but the PKGBUILD expects $EXPECTED_DEB"

UNIT_SRC="packaging/systemd/hyprlayer-server.service"
[[ -f "$UNIT_SRC" ]] || die "missing $UNIT_SRC"
UNIT_SHA="$(sha256sum "$UNIT_SRC" | awk '{print $1}')"

AUR_DIR="$BUILD_DIR/aur-out"
rm -rf "$AUR_DIR"
mkdir -p "$AUR_DIR"
# Both PKGBUILDs list the unit as a local source= entry, so it has to sit next
# to each one for makepkg and for the AUR checkout.
cp "$UNIT_SRC" "$AUR_DIR/"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@SHA256@/$DEB_SHA/g" -e "s/@SHA256_UNIT@/$UNIT_SHA/g" \
  packaging/aur/PKGBUILD.in > "$AUR_DIR/PKGBUILD"
if grep -q '@VERSION@\|@SHA256@\|@SHA256_UNIT@' "$AUR_DIR/PKGBUILD"; then
  die "PKGBUILD still has unsubstituted placeholders"
fi
(cd "$AUR_DIR" && makepkg --printsrcinfo > .SRCINFO)
[[ -s "$AUR_DIR/.SRCINFO" ]] || die "makepkg produced an empty .SRCINFO"

SERVER_BIN="$BUILD_DIR/src-tauri/binaries/hyprlayer-server-x86_64-unknown-linux-gnu"
# Staged by beforeBuildCommand during `tauri build`. Not optional: the desktop
# cannot start without a server beside it, and the AUR package below ships it.
[[ -f "$SERVER_BIN" ]] \
  || die "no hyprlayer-server sidecar at $SERVER_BIN — did beforeBuildCommand run?"

step "Rendering hyprlayer-server AUR PKGBUILD + .SRCINFO"
SERVER_SHA="$(sha256sum "$SERVER_BIN" | awk '{print $1}')"
# The server package follows the server's own version, not the app's (ADR-0009).
SERVER_VERSION="$(sed -nE 's/^version = "(.*)"$/\1/p' crates/hyprlayer-server/Cargo.toml)"
[[ -n "$SERVER_VERSION" ]] || die "no version line in crates/hyprlayer-server/Cargo.toml"

AUR_SERVER_DIR="$BUILD_DIR/aur-server-out"
rm -rf "$AUR_SERVER_DIR"
mkdir -p "$AUR_SERVER_DIR"
cp "$UNIT_SRC" "$AUR_SERVER_DIR/"
sed -e "s/@VERSION@/$SERVER_VERSION/g" \
    -e "s/@SHA256_SERVER@/$SERVER_SHA/g" \
    -e "s/@SHA256_UNIT@/$UNIT_SHA/g" \
  packaging/aur/PKGBUILD-server.in > "$AUR_SERVER_DIR/PKGBUILD"
if grep -q '@VERSION@\|@SHA256_SERVER@\|@SHA256_UNIT@' "$AUR_SERVER_DIR/PKGBUILD"; then
  die "server PKGBUILD still has unsubstituted placeholders"
fi
(cd "$AUR_SERVER_DIR" && makepkg --printsrcinfo > .SRCINFO)
[[ -s "$AUR_SERVER_DIR/.SRCINFO" ]] || die "makepkg produced an empty server .SRCINFO"

echo ""
echo "SERVER_BIN=$SERVER_BIN"
echo "SERVER_SHA=$SERVER_SHA"
echo "DEB=$DEB"
echo "RPM=$RPM"
echo "AUR_DIR=$AUR_DIR"
echo "AUR_SERVER_DIR=$AUR_SERVER_DIR"
