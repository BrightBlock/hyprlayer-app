#!/usr/bin/env bash
# Build the Linux x64 AppImage on this machine (the "nexus" builder), driven
# over SSH by scripts/release-local.sh on the Mac.
#
# The Mac exports the exact release-tag source with 'git archive' and ships it
# here as a tarball, so this build matches what the Mac publishes byte for byte,
# independent of any working clone on this box. We build in a clean throwaway
# dir and never touch the developer's own checkout.
#
# Updater artifacts are deliberately disabled: the ed25519 updater signing key
# stays on the Mac, which signs the copied-down AppImage itself. This machine
# only needs to produce the raw <Name>_<version>_amd64.AppImage. That raw file
# IS the updater payload on Linux — tauri-plugin-updater writes the downloaded
# bytes straight over the running AppImage (see its install_appimage), so no
# tarball wrapper is involved.
#
# On success the LAST line printed is "APPIMAGE=<full path>", which the Mac
# orchestrator parses to know what to copy back.
#
# Builder prerequisites (Arch names in parens):
#   * Rust stable (rustup), Node 22+ and pnpm — resolved through a login shell,
#     so mise/nvm/volta shims are fine
#   * webkit2gtk-4.1, gtk3, libayatana-appindicator/libappindicator-gtk3,
#     librsvg, openssl, base-devel
#   * curl + fuse2 (or a kernel with FUSE): the AppImage bundler downloads
#     linuxdeploy and runs it with --appimage-extract-and-run
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
for tool in cargo pnpm node tar; do
  command -v "$tool" >/dev/null || die "missing required tool on the Linux builder: $tool"
done

[[ -f "$SRC_TARBALL" ]] || die "source tarball not found: $SRC_TARBALL"

step "Resetting build dir $BUILD_DIR"
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR"

step "Extracting source tarball"
tar -xzf "$SRC_TARBALL" -C "$BUILD_DIR"

cd "$BUILD_DIR"

step "Disabling updater artifacts (signing happens on the Mac)"
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

step "tauri build --bundles appimage"
export CARGO_TARGET_DIR="$TARGET_DIR"
# linuxdeploy can't mount its own AppImage over a bare SSH session (no FUSE
# device); extract-and-run is the documented escape hatch and is harmless
# otherwise.
export APPIMAGE_EXTRACT_AND_RUN=1
# linuxdeploy vendors its own ancient binutils `strip`, which aborts on the
# `.relr.dyn` section modern toolchains emit ("unknown type [0x13]") — every
# system .so it copies in fails, and the whole bundle step fails with it. We
# lose nothing by skipping it: [profile.release] already sets `strip = true`
# for our own binary, and the copied system libraries are the distro's already-
# stripped ones.
export NO_STRIP=1
pnpm tauri build --bundles appimage

step "Locating AppImage"
APPIMAGE_DIR="$TARGET_DIR/release/bundle/appimage"
APPIMAGE="$(ls -t "$APPIMAGE_DIR"/*.AppImage 2>/dev/null | head -n1 || true)"
[[ -n "$APPIMAGE" ]] || die "no *.AppImage found under $APPIMAGE_DIR"

echo ""
echo "APPIMAGE=$APPIMAGE"
