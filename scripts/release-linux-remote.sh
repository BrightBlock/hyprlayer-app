#!/usr/bin/env bash
# Build the Linux x64 distro packages on this machine (the "nexus" builder),
# driven over SSH by scripts/release-local.sh on the Mac.
#
# The Mac exports the exact release-tag source with 'git archive' and ships it
# here as a tarball, so this build matches what the Mac publishes byte for byte,
# independent of any working clone on this box. We build in a clean throwaway
# dir and never touch the developer's own checkout.
#
# Outputs a .deb and an .rpm, plus the rendered AUR PKGBUILD/.SRCINFO pair for
# hyprlayer-desktop-bin. Arch is the primary Linux target, and this box IS an
# Arch box, so the ELF we produce links Arch's webkit2gtk-4.1 — the AUR package
# just unpacks the .deb. That also sets the floor for the .deb/.rpm: they need a
# glibc no older than this builder's (see docs/RELEASING.md).
#
# The .SRCINFO is generated here rather than on the Mac because `makepkg` is the
# only authority on that format, and it only exists on Arch.
#
# Updater artifacts are deliberately disabled: distro packages are owned by the
# package manager, so the app never self-installs on Linux and there is nothing
# to ed25519-sign.
#
# On success the LAST lines printed are DEB=, RPM= and AUR_DIR=, which the Mac
# orchestrator parses to know what to copy back.
#
# Builder prerequisites (Arch names):
#   * Rust stable (rustup), Node 22+ and pnpm — resolved through a login shell,
#     so mise/nvm/volta shims are fine
#   * webkit2gtk-4.1, gtk3, librsvg, base-devel
# Both bundlers are pure Rust: no dpkg, no rpmbuild, no FUSE.
# It does NOT need the signing key or any Git credentials.
#
# Invoked from the Mac as (paths default under the builder's home dir, so the
# orchestrator just scps the tarball to ~ and runs this with no args):
#   bash -lc '~/release-linux-remote.sh'
set -euo pipefail

# Source tarball the Mac scp'd into the builder's home dir, and a throwaway
# build dir. Defaults keep the Mac side from having to resolve the remote home.
SRC_TARBALL="${1:-$HOME/hyprlayer-src.tgz}"
BUILD_DIR="${2:-$HOME/hyprlayer-release-build}"
# Persistent cargo target dir (outside BUILD_DIR, which is wiped each run) so
# crate compilation is cached across releases; only the app's own crates
# recompile when the source changes.
TARGET_DIR="${3:-$HOME/.hyprlayer-linux-target}"

step() { printf '==> %s\n' "$*"; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }

# A login shell should have put the toolchains on PATH; fail loudly here rather
# than 30 seconds into a build with a confusing "command not found".
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
# Surgical text replace rather than a JSON round-trip, mirroring the Windows
# helper: rewriting the file through a JSON encoder would reorder keys and
# re-escape the CSP. A literal swap is safe.
CONF="src-tauri/tauri.conf.json"
grep -q '"createUpdaterArtifacts"[[:space:]]*:[[:space:]]*true' "$CONF" \
  || echo "warning: createUpdaterArtifacts:true not found; build will need a signing key and likely fail" >&2
tmp="$(mktemp)"
sed -E 's/"createUpdaterArtifacts"[[:space:]]*:[[:space:]]*true/"createUpdaterArtifacts": false/' \
  "$CONF" > "$tmp" && mv "$tmp" "$CONF"

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
# The PKGBUILD's source URL is built from pkgver, so its filename must be the
# one we're about to publish — catch a bundler rename here, not in the AUR.
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
