#!/usr/bin/env bash
# Runs inside the linux builder image: /in holds the source tarball, /out
# receives the packages, the server and both AUR dirs, /target is the warm
# cargo target dir. run.sh renders each .SRCINFO afterwards, on Arch.
set -euo pipefail

TRIPLE="x86_64-unknown-linux-gnu"
SRC_TARBALL="/in/hyprlayer-src.tgz"
BUILD_DIR="/build"
OUT="/out"
export CARGO_TARGET_DIR="/target"

step() { printf '==> %s\n' "$*"; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }

[[ -f "$SRC_TARBALL" ]] || die "source tarball not found: $SRC_TARBALL"

# One build at a time in /target: --touch below only orders runs that don't overlap.
exec 9>"$CARGO_TARGET_DIR/.hyprlayer-build.lock"
flock -n 9 || { step "Waiting for another build in /target"; flock 9; }

step "Extracting source tarball"
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR"
# Dev stages build into /target too, and cargo trusts source mtimes.
tar -xzf "$SRC_TARBALL" -C "$BUILD_DIR" --touch
cd "$BUILD_DIR"

step "Disabling updater artifacts (Linux installs update through their package manager)"
CONF="src-tauri/tauri.conf.json"
grep -q '"createUpdaterArtifacts"[[:space:]]*:[[:space:]]*true' "$CONF" \
  || echo "warning: createUpdaterArtifacts:true not found; build will need a signing key and likely fail" >&2
sed -i -E 's/"createUpdaterArtifacts"[[:space:]]*:[[:space:]]*true/"createUpdaterArtifacts": false/' "$CONF"

step "Verifying hyprlayer-models + hyprlayer-core build standalone for $TRIPLE"
cargo check -p hyprlayer-models -p hyprlayer-core --target "$TRIPLE"   # specta-less: the app enables it, embedders don't

step "pnpm install (frozen lockfile)"
pnpm install --frozen-lockfile --store-dir /pnpm-store

step "tauri build --target $TRIPLE --bundles deb,rpm"
HYPRLAYER_SIDECAR_TARGET="$TRIPLE" pnpm tauri build --target "$TRIPLE" --bundles deb,rpm

step "Locating packages"
VERSION="$(node -p 'require("./package.json").version')"
BUNDLE="$CARGO_TARGET_DIR/$TRIPLE/release/bundle"
DEB="$BUNDLE/deb/Hyprlayer_${VERSION}_amd64.deb"
RPM="$(ls "$BUNDLE"/rpm/Hyprlayer-"$VERSION"-*.x86_64.rpm 2>/dev/null | head -n1 || true)"
[[ -f "$DEB" ]] || die "no $(basename "$DEB") under $BUNDLE/deb"
[[ -n "$RPM" ]] || die "no Hyprlayer-$VERSION rpm under $BUNDLE/rpm"

step "Verifying the deb carries hyprlayer-server beside the app"
# supervisor.rs resolves the server as a sibling of current_exe(), so a sidecar
# staged anywhere but /usr/bin is an app that cannot start once installed.
DEB_PATHS="$(bsdtar -xOf "$DEB" 'data.tar*' | bsdtar -tf - || true)"
if ! printf '%s\n' "$DEB_PATHS" | grep -qE '(^|\./)usr/bin/hyprlayer-server$'; then
  printf '%s\n' "$DEB_PATHS" | grep -E 'hyprlayer' | head -20 >&2 || true
  die "the deb has no usr/bin/hyprlayer-server (paths containing 'hyprlayer' listed above)"
fi

SERVER_BIN="src-tauri/binaries/hyprlayer-server-$TRIPLE"
# Staged by beforeBuildCommand during `tauri build`. Not optional: the desktop
# cannot start without a server beside it, and the AUR package below ships it.
[[ -f "$SERVER_BIN" ]] \
  || die "no hyprlayer-server sidecar at $SERVER_BIN — did beforeBuildCommand run?"

step "Rendering both AUR PKGBUILDs"
DEB_SHA="$(sha256sum "$DEB" | awk '{print $1}')"
SERVER_SHA="$(sha256sum "$SERVER_BIN" | awk '{print $1}')"
UNIT_SRC="packaging/systemd/hyprlayer-server.service"
[[ -f "$UNIT_SRC" ]] || die "missing $UNIT_SRC"
UNIT_SHA="$(sha256sum "$UNIT_SRC" | awk '{print $1}')"
# The server package follows the server's own version, not the app's (ADR-0009).
SERVER_VERSION="$(sed -nE 's/^version = "(.*)"$/\1/p' crates/hyprlayer-server/Cargo.toml)"
[[ -n "$SERVER_VERSION" ]] || die "no version line in crates/hyprlayer-server/Cargo.toml"

rm -rf "$OUT/aur" "$OUT/aur-server"
mkdir -p "$OUT/aur" "$OUT/aur-server"
# Both PKGBUILDs list the unit as a local source= entry, so it has to sit next
# to each one for makepkg and for the AUR checkout.
cp "$UNIT_SRC" "$OUT/aur/"
cp "$UNIT_SRC" "$OUT/aur-server/"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@SHA256@/$DEB_SHA/g" -e "s/@SHA256_UNIT@/$UNIT_SHA/g" \
  packaging/aur/PKGBUILD.in > "$OUT/aur/PKGBUILD"
sed -e "s/@VERSION@/$SERVER_VERSION/g" -e "s/@SHA256_SERVER@/$SERVER_SHA/g" -e "s/@SHA256_UNIT@/$UNIT_SHA/g" \
  packaging/aur/PKGBUILD-server.in > "$OUT/aur-server/PKGBUILD"
if grep -q '@VERSION@\|@SHA256@\|@SHA256_SERVER@\|@SHA256_UNIT@' "$OUT/aur/PKGBUILD" "$OUT/aur-server/PKGBUILD"; then
  die "a PKGBUILD still has unsubstituted placeholders"
fi

cp "$DEB" "$RPM" "$OUT/"
cp "$SERVER_BIN" "$OUT/hyprlayer-server-$TRIPLE"

echo ""
echo "SERVER_BIN=$OUT/hyprlayer-server-$TRIPLE"
echo "SERVER_SHA=$SERVER_SHA"
echo "DEB=$OUT/$(basename "$DEB")"
echo "RPM=$OUT/$(basename "$RPM")"
echo "AUR_DIR=$OUT/aur"
echo "AUR_SERVER_DIR=$OUT/aur-server"
