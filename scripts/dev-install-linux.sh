#!/usr/bin/env bash
# Build and install "Hyprlayer Dev" on Linux — a desktop install that tracks
# origin/main, SIDE BY SIDE with the released app that pacman/apt/dnf owns.
# (macOS: scripts/dev-install-macos.sh.)
#
# Why a separate install rather than `pnpm tauri dev`:
#   * it survives a reboot and shows up in the app launcher, so main can be
#     dogfooded all day instead of only while a terminal holds the dev server
#   * it is a release-profile build (no vite dev server, no HMR), so what you're
#     testing is what a release would feel like
#
# Three things keep it from colliding with the released install:
#   * identifier  ai.brightblock.hyprlayer.main  → its own SQLite DB, settings,
#     and webview storage under ~/.local/share and ~/.config. Nothing this build
#     migrates can corrupt the stable app's database.
#   * it installs under ~/.local, never /usr, so pacman still owns the release
#   * a distinct binary name (Hyprlayer-Dev) → its own window class, so the
#     compositor and taskbar treat the two as different apps
#
# The identifier says ".main" while the app is called "Hyprlayer Dev" on purpose:
# ".dev" already belongs to the `verify` skill, which launches
# `tauri dev --config src-tauri/tauri.dev.conf.json` under
# ai.brightblock.hyprlayer.dev. Sharing that identifier would let a running
# install hijack a verify run through the single-instance plugin, so this build
# gets its own — named after the branch it tracks.
#
# What lands where:
#   binary   ~/.local/bin/Hyprlayer-Dev          (bare ELF — Linux needs no bundle)
#   launcher ~/.local/share/applications/Hyprlayer-Dev.desktop
#   icons    ~/.local/share/icons/hicolor/*/apps/Hyprlayer-Dev.png
#   data     ~/.local/share/ai.brightblock.hyprlayer.main
#   settings ~/.config/ai.brightblock.hyprlayer.main
#
# The build happens in a throwaway clone under ~/.cache (never this working
# tree), so it can run while you have uncommitted work in progress, and it
# always builds exactly what's on the remote rather than whatever you had
# checked out. The cargo target dir is persistent, so only the first build is
# cold; after that it's an incremental rebuild of the app's own crates.
#
# Updater note: the in-app updater is notify-only on Linux (see src/lib/updater.ts),
# so this build can never install over itself. Between releases it otherwise
# reports the same version as the release it was cut from, so the version
# carries a +main.<sha> suffix (Settings → Updates reads e.g. 0.5.10+main.d39276a)
# to say which commit you're on. Pass --no-version-suffix for a bare x.y.z.
#
# Usage:
#   ./scripts/dev-install-linux.sh              # fetch origin/main, build if it moved, install
#   ./scripts/dev-install-linux.sh --force      # rebuild even if main hasn't moved
#   ./scripts/dev-install-linux.sh --ref <ref>  # track something else (a branch, tag, SHA)
#   ./scripts/dev-install-linux.sh --no-fetch   # use what's already fetched (offline)
#   ./scripts/dev-install-linux.sh --uninstall  # remove the install (keeps its data dir)
#
# It also runs with no checkout at all — anyone with repo access can pull it
# straight from GitHub, where `gh` supplies the auth the private repo needs:
#
#   bash <(gh api repos/BrightBlock/hyprlayer-app/contents/scripts/dev-install-linux.sh \
#            -H 'Accept: application/vnd.github.raw')            # add flags after the `)`
#
# Standalone it clones the repo itself; the flags above all still apply, and
# HYPRLAYER_DEV_ORIGIN overrides the clone URL (a fork, or a local mirror).
#
# Prerequisites: the same toolchain the Linux release builder needs, minus every
# secret — Rust, Node + pnpm, and webkit2gtk-4.1 / gtk3 / librsvg. No signing
# key, no publish credentials.
set -euo pipefail

# Where the build clone comes from. Run from a checkout, that's the working tree
# beside this script (fast — no network for the object transfer). Run standalone
# — piped from `gh api`, so $0 is "bash" or a /dev/fd pipe — there is no tree to
# clone, and the repo is fetched from GitHub instead. The `scripts/<self>` probe
# is what tells the two apart: a real checkout has this file under it.
SELF="$0"
ORIGIN_SLUG="BrightBlock/hyprlayer-app"
BOOTSTRAP="bash <(gh api repos/$ORIGIN_SLUG/contents/scripts/dev-install-linux.sh -H 'Accept: application/vnd.github.raw')"
REPO=""
if self_dir="$(cd "$(dirname -- "$SELF")/.." 2>/dev/null && pwd)" \
  && [[ -e "$self_dir/.git" && -f "$self_dir/scripts/$(basename -- "$SELF")" ]]; then
  REPO="$self_dir"
fi

[[ "$(uname -s)" == Linux ]] || {
  printf 'error: this is the Linux installer; on macOS use scripts/dev-install-macos.sh\n' >&2
  exit 1
}

# ─── config ─────────────────────────────────────────────────────────
CACHE_DIR="${HYPRLAYER_DEV_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/hyprlayer-dev}"
SRC_DIR="$CACHE_DIR/src"
TARGET_DIR="$CACHE_DIR/target"
STAMP="$CACHE_DIR/installed"
PREFIX="${HYPRLAYER_DEV_PREFIX:-$HOME/.local}"
APP_ID="ai.brightblock.hyprlayer.main"
APP_NAME="Hyprlayer Dev"
# The binary name doubles as the icon name, the .desktop basename, and the
# window class — keep the three spellings identical or the taskbar shows a
# blank icon.
BIN_NAME="Hyprlayer-Dev"

INSTALLED_BIN="$PREFIX/bin/$BIN_NAME"
DESKTOP_FILE="$PREFIX/share/applications/$BIN_NAME.desktop"
DATA_DIR="$HOME/.local/share/$APP_ID"
CONFIG_DIR="$HOME/.config/$APP_ID"
# hicolor size : source file, shared by install and uninstall so the two can't
# drift. 128x128@2x.png is a 256px image.
ICON_MAP=("32x32:32x32.png" "128x128:128x128.png" "256x256:128x128@2x.png")
INSTALL_PATHS=("$INSTALLED_BIN" "$DESKTOP_FILE")
for icon in "${ICON_MAP[@]}"; do
  INSTALL_PATHS+=("$PREFIX/share/icons/hicolor/${icon%%:*}/apps/$BIN_NAME.png")
done

REF="origin/main"
FORCE=0
FETCH=1
UNINSTALL=0
VERSION_SUFFIX=1

step() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
info() { printf '    %s\n' "$*"; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

# The header doubles as the help text, but standalone there is no file to read
# it back out of ($SELF is a pipe), so fall back to the flag list.
usage() {
  if [[ -f "$SELF" ]]; then
    sed -n '2,/^set -euo/p' "$SELF" | sed '$d'
  else
    printf '%s\n' \
      'dev-install-linux.sh - build and install "Hyprlayer Dev" from origin/main' \
      '' \
      '  --force               rebuild even if the ref has not moved' \
      '  --ref <ref>           track something else (a branch, tag, SHA)' \
      '  --no-fetch            use what is already fetched (offline)' \
      '  --no-version-suffix   build a bare x.y.z' \
      '  --uninstall           remove the install (keeps its data dir)'
  fi
}

refresh_caches() {
  command -v update-desktop-database >/dev/null \
    && update-desktop-database -q "$PREFIX/share/applications" 2>/dev/null || true
  command -v gtk-update-icon-cache >/dev/null \
    && gtk-update-icon-cache -qtf "$PREFIX/share/icons/hicolor" 2>/dev/null || true
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --force)     FORCE=1; shift ;;
    --no-fetch)  FETCH=0; shift ;;
    --uninstall) UNINSTALL=1; shift ;;
    --no-version-suffix) VERSION_SUFFIX=0; shift ;;
    --ref)       REF="${2:?--ref needs a value}"; shift 2 ;;
    -h|--help)   usage; exit 0 ;;
    *)           die "unknown argument: $1" ;;
  esac
done

# ─── uninstall ──────────────────────────────────────────────────────
if [[ $UNINSTALL -eq 1 ]]; then
  step "Removing $APP_NAME"
  rm -fv "${INSTALL_PATHS[@]}" "$STAMP"
  refresh_caches
  info "Kept (delete by hand if you want a clean slate):"
  info "  build cache  $CACHE_DIR"
  info "  app data     $DATA_DIR"
  info "  settings     $CONFIG_DIR"
  exit 0
fi

for tool in git cargo pnpm node install; do
  command -v "$tool" >/dev/null || die "missing required tool: $tool"
done

# ─── 1. a throwaway clone that tracks the remote ────────────────────
# From a checkout: cloned from the local repo, then repointed at the real origin
# so `origin/main` means the same thing here as it does in the working tree.
# Standalone: cloned from GitHub through `gh repo clone`, which carries the gh
# token and honours the user's ssh/https preference — a bare `git clone` of a
# private repo only works if they have already run `gh auth setup-git`.
if [[ ! -d "$SRC_DIR/.git" ]]; then
  step "Creating build clone at $SRC_DIR"
  mkdir -p "$CACHE_DIR"
  rm -rf "$SRC_DIR"
  if [[ -n "$REPO" ]]; then
    git clone --quiet "$REPO" "$SRC_DIR"
    git -C "$SRC_DIR" remote set-url origin "$(git -C "$REPO" remote get-url origin)"
  elif [[ -z "${HYPRLAYER_DEV_ORIGIN:-}" ]] && command -v gh >/dev/null; then
    info "source: $ORIGIN_SLUG via gh"
    gh repo clone "$ORIGIN_SLUG" "$SRC_DIR" -- --quiet \
      || die "clone failed — is gh authenticated for $ORIGIN_SLUG? (gh auth status)"
  else
    info "source: ${HYPRLAYER_DEV_ORIGIN:-https://github.com/$ORIGIN_SLUG.git}"
    git clone --quiet "${HYPRLAYER_DEV_ORIGIN:-https://github.com/$ORIGIN_SLUG.git}" "$SRC_DIR"
  fi
fi

if [[ $FETCH -eq 1 ]]; then
  step "Fetching $REF"
  git -C "$SRC_DIR" fetch --quiet --prune origin \
    || die "fetch failed (try --no-fetch to build what's already here)"
fi

SHA="$(git -C "$SRC_DIR" rev-parse --verify "$REF^{commit}" 2>/dev/null)" \
  || die "no such ref in the build clone: $REF"
SHORT="$(git -C "$SRC_DIR" rev-parse --short "$SHA")"
SUBJECT="$(git -C "$SRC_DIR" log -1 --format=%s "$SHA")"

if [[ $FORCE -eq 0 && -x "$INSTALLED_BIN" && -f "$STAMP" ]] && grep -qx "$SHA" "$STAMP"; then
  step "Already on $REF ($SHORT — $SUBJECT)"
  info "Nothing to build. Use --force to rebuild anyway."
  exit 0
fi

# ─── 2. check out the exact commit ──────────────────────────────────
# `clean -fd` (no -x) drops stray tracked-turned-untracked files but keeps the
# gitignored node_modules/ and dist/, which is what makes reruns cheap.
step "Checking out $SHORT — $SUBJECT"
git -C "$SRC_DIR" checkout --quiet --detach "$SHA"
git -C "$SRC_DIR" reset --hard --quiet "$SHA"
git -C "$SRC_DIR" clean -qfd

cd "$SRC_DIR"

# ─── 3. config overlay ──────────────────────────────────────────────
# Written into the build clone (not this repo) so nothing here needs committing.
# createUpdaterArtifacts is forced off: signing them needs the release key, and
# a Linux build never self-installs anyway.
VERSION="$(node -p 'require("./package.json").version')"
if [[ $VERSION_SUFFIX -eq 1 ]]; then
  BUILD_VERSION="$VERSION+main.$SHORT"
else
  BUILD_VERSION="$VERSION"
fi

OVERLAY="src-tauri/tauri.devinstall.conf.json"
# The windows array is carried over WHOLESALE from tauri.linux.conf.json with
# only `title` swapped. Tauri's config merge is recursive for objects but
# REPLACES arrays, so an overlay that names `app.windows` at all discards every
# key it doesn't restate — which is how the OS title bar came back the first
# time (`decorations: false` lost). The assert below fails the build if that key
# ever moves rather than silently shipping the wrong chrome.
APP_NAME="$APP_NAME" APP_ID="$APP_ID" BUILD_VERSION="$BUILD_VERSION" \
  node -e '
    const fs = require("fs");
    const conf = JSON.parse(fs.readFileSync("src-tauri/tauri.linux.conf.json", "utf8"));
    const windows = conf.app?.windows;
    if (!Array.isArray(windows) || !windows.length) {
      throw new Error("tauri.linux.conf.json has no app.windows to carry over");
    }
    const w = windows[0];
    if (w.decorations !== false) {
      throw new Error("expected decorations:false — did the window config move?");
    }
    fs.writeFileSync("src-tauri/tauri.devinstall.conf.json", JSON.stringify({
      $schema: "https://schema.tauri.app/config/2",
      productName: process.env.APP_NAME,
      identifier: process.env.APP_ID,
      version: process.env.BUILD_VERSION,
      app: { windows: [{ ...w, title: process.env.APP_NAME }, ...windows.slice(1)] },
      bundle: { createUpdaterArtifacts: false },
    }, null, 2) + "\n");
  '

# ─── 4. build ───────────────────────────────────────────────────────
step "pnpm install (frozen lockfile)"
pnpm install --frozen-lockfile

step "Building $APP_NAME $BUILD_VERSION (release profile, no bundler)"
info "target dir: $TARGET_DIR — first run is a cold build, later runs are incremental"
export CARGO_TARGET_DIR="$TARGET_DIR"
pnpm tauri build --no-bundle --config "$OVERLAY"

BUILT_BIN="$TARGET_DIR/release/Hyprlayer"
[[ -x "$BUILT_BIN" ]] || die "expected binary not found: $BUILT_BIN"

# ─── 5. install under ~/.local ──────────────────────────────────────
step "Installing to $PREFIX"
# Stage then rename: writing straight over the target is ETXTBSY if the app is
# running, and rename() swaps the directory entry while the live process keeps
# its old inode — so refreshing while the dev app is open just works.
mkdir -p "$PREFIX/bin"
install -m 755 "$BUILT_BIN" "$INSTALLED_BIN.new"
mv -f "$INSTALLED_BIN.new" "$INSTALLED_BIN"

# Same three sizes the .deb ships.
for icon in "${ICON_MAP[@]}"; do
  mkdir -p "$PREFIX/share/icons/hicolor/${icon%%:*}/apps"
  install -m 644 "src-tauri/icons/${icon##*:}" \
    "$PREFIX/share/icons/hicolor/${icon%%:*}/apps/$BIN_NAME.png"
done

mkdir -p "$(dirname "$DESKTOP_FILE")"
cat > "$DESKTOP_FILE" <<DESKTOP
[Desktop Entry]
Type=Application
Name=$APP_NAME
Comment=Hyprlayer built from origin/main — testing build, not the release
Categories=Development;
Exec=$BIN_NAME
Icon=$BIN_NAME
StartupWMClass=$BIN_NAME
Terminal=false
DESKTOP

refresh_caches

printf '%s\n' "$SHA" > "$STAMP"

step "Installed $APP_NAME $BUILD_VERSION"
info "commit    $SHORT — $SUBJECT"
info "path      $INSTALLED_BIN"
info "launch    $BIN_NAME   (or \"$APP_NAME\" in the app launcher)"
info "data      $DATA_DIR  (separate from the released app)"
if [[ -n "$REPO" ]]; then
  info "refresh   $REPO/scripts/dev-install-linux.sh"
else
  info "refresh   $BOOTSTRAP"
fi
