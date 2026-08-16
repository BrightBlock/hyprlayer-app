#!/usr/bin/env bash
# Build and install "Hyprlayer Dev" on macOS — a desktop install that tracks
# origin/main, SIDE BY SIDE with the released app from the Homebrew cask.
# (Linux: scripts/dev-install-linux.sh.)
#
# Why a separate install rather than `pnpm tauri dev`:
#   * it survives a reboot and shows up in Spotlight and Launchpad, so main can
#     be dogfooded all day instead of only while a terminal holds the dev server
#   * it is a release-profile build (no vite dev server, no HMR), so what you're
#     testing is what a release would feel like
#
# Three things keep it from colliding with the released install:
#   * identifier  ai.brightblock.hyprlayer.main  → its own SQLite DB, settings,
#     and webview storage under ~/Library. Nothing this build migrates can
#     corrupt the stable app's database.
#   * it installs into ~/Applications, never /Applications, so the cask still
#     owns the release
#   * a distinct bundle name (Hyprlayer Dev.app) → its own Dock tile and
#     app-switcher entry
#
# The identifier says ".main" while the app is called "Hyprlayer Dev" on purpose:
# ".dev" already belongs to the `verify` skill, which launches
# `tauri dev --config src-tauri/tauri.dev.conf.json` under
# ai.brightblock.hyprlayer.dev. Sharing that identifier would let a running
# install hijack a verify run through the single-instance plugin, so this build
# gets its own — named after the branch it tracks.
#
# What lands where:
#   bundle   ~/Applications/Hyprlayer Dev.app
#   data     ~/Library/Application Support/ai.brightblock.hyprlayer.main
#            (macOS keeps settings under the same identifier)
#
# The build happens in a throwaway clone under ~/.cache (never this working
# tree), so it can run while you have uncommitted work in progress, and it
# always builds exactly what's on the remote rather than whatever you had
# checked out. The cargo target dir is persistent, so only the first build is
# cold; after that it's an incremental rebuild of the app's own crates.
#
# Signing: this build is deliberately left UNSIGNED. APPLE_SIGNING_IDENTITY and
# the notary-key variables are unset before the build so a dev install never
# reaches for the Developer ID cert or triggers notarization, even if your shell
# has them exported from a release run. A locally-built .app carries no
# quarantine attribute, so Gatekeeper lets it run. Updater artifacts are off for
# the same reason — signing those needs the ed25519 release key.
#
# Updater note: unlike Linux, a macOS build CAN self-install an update. The
# +main.<sha> version below is not a release version, so the manifest's x.y.z
# never compares as newer and the nudge stays quiet. That suffix is also what
# tells you which commit you're on (Settings → Updates reads e.g.
# 0.5.10+main.d39276a). Apple documents CFBundleShortVersionString as up to
# three period-separated integers; nothing enforces it for a local run, but
# --no-version-suffix gives you a bare x.y.z if anything downstream objects.
#
# Usage:
#   ./scripts/dev-install-macos.sh              # fetch origin/main, build if it moved, install
#   ./scripts/dev-install-macos.sh --force      # rebuild even if main hasn't moved
#   ./scripts/dev-install-macos.sh --ref <ref>  # track something else (a branch, tag, SHA)
#   ./scripts/dev-install-macos.sh --no-fetch   # use what's already fetched (offline)
#   ./scripts/dev-install-macos.sh --uninstall  # remove the install (keeps its data dir)
#
# Prerequisites: Rust, Node + pnpm, and the Xcode command line tools. No signing
# key, no notary key, no publish credentials.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"

[[ "$(uname -s)" == Darwin ]] || {
  printf 'error: this is the macOS installer; on Linux use scripts/dev-install-linux.sh\n' >&2
  exit 1
}

# ─── config ─────────────────────────────────────────────────────────
CACHE_DIR="${HYPRLAYER_DEV_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/hyprlayer-dev}"
SRC_DIR="$CACHE_DIR/src"
TARGET_DIR="$CACHE_DIR/target"
STAMP="$CACHE_DIR/installed"
# ~/Applications needs no sudo and is indexed by Spotlight and Launchpad exactly
# like /Applications.
APPDIR="${HYPRLAYER_DEV_APPDIR:-$HOME/Applications}"
APP_ID="ai.brightblock.hyprlayer.main"
APP_NAME="Hyprlayer Dev"

APP_BUNDLE="$APPDIR/$APP_NAME.app"
DATA_DIR="$HOME/Library/Application Support/$APP_ID"

REF="origin/main"
FORCE=0
FETCH=1
UNINSTALL=0
VERSION_SUFFIX=1

step() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
info() { printf '    %s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --force)     FORCE=1; shift ;;
    --no-fetch)  FETCH=0; shift ;;
    --uninstall) UNINSTALL=1; shift ;;
    --no-version-suffix) VERSION_SUFFIX=0; shift ;;
    --ref)       REF="${2:?--ref needs a value}"; shift 2 ;;
    -h|--help)   sed -n '2,/^set -euo/p' "$0" | sed '$d'; exit 0 ;;
    *)           die "unknown argument: $1" ;;
  esac
done

# ─── uninstall ──────────────────────────────────────────────────────
if [[ $UNINSTALL -eq 1 ]]; then
  step "Removing $APP_NAME"
  rm -rfv "$APP_BUNDLE" "$STAMP"
  info "Kept (delete by hand if you want a clean slate):"
  info "  build cache  $CACHE_DIR"
  info "  app data     $DATA_DIR"
  exit 0
fi

for tool in git cargo pnpm node; do
  command -v "$tool" >/dev/null || die "missing required tool: $tool"
done

# ─── 1. a throwaway clone that tracks the remote ────────────────────
# Cloned from the local repo (fast, no network for the object transfer), then
# repointed at the real origin so `origin/main` means the same thing here as it
# does in the working tree.
if [[ ! -d "$SRC_DIR/.git" ]]; then
  step "Creating build clone at $SRC_DIR"
  mkdir -p "$CACHE_DIR"
  rm -rf "$SRC_DIR"
  git clone --quiet "$REPO" "$SRC_DIR"
  git -C "$SRC_DIR" remote set-url origin "$(git -C "$REPO" remote get-url origin)"
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

if [[ $FORCE -eq 0 && -d "$APP_BUNDLE" && -f "$STAMP" ]] && grep -qx "$SHA" "$STAMP"; then
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
# createUpdaterArtifacts is forced off: signing them needs the release key.
VERSION="$(node -p 'require("./package.json").version')"
if [[ $VERSION_SUFFIX -eq 1 ]]; then
  BUILD_VERSION="$VERSION+main.$SHORT"
else
  BUILD_VERSION="$VERSION"
fi

OVERLAY="src-tauri/tauri.devinstall.conf.json"
# The windows array is carried over WHOLESALE from the config Tauri itself would
# resolve for macOS, with only `title` swapped. Tauri's config merge is
# recursive for objects but REPLACES arrays, so an overlay that names
# `app.windows` at all discards every key it doesn't restate — that is how the
# OS title bar came back on the Linux side once (`decorations: false` lost).
# Here the keys at stake are titleBarStyle/hiddenTitle/trafficLightPosition; the
# assert below fails the build if they ever move rather than silently shipping
# the wrong chrome. There is no tauri.macos.conf.json today, so the base config
# supplies the window — but read the override if one is ever added.
APP_NAME="$APP_NAME" APP_ID="$APP_ID" BUILD_VERSION="$BUILD_VERSION" \
  node -e '
    const fs = require("fs");
    const read = (p) => JSON.parse(fs.readFileSync(p, "utf8"));
    let windows = read("src-tauri/tauri.conf.json").app?.windows;
    if (fs.existsSync("src-tauri/tauri.macos.conf.json")) {
      const override = read("src-tauri/tauri.macos.conf.json").app?.windows;
      if (Array.isArray(override) && override.length) windows = override;
    }
    if (!Array.isArray(windows) || !windows.length) {
      throw new Error("no app.windows found for macos");
    }
    const w = windows[0];
    if (w.titleBarStyle !== "Overlay") {
      throw new Error("expected titleBarStyle:Overlay — did the window config move?");
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

step "Building $APP_NAME $BUILD_VERSION (release profile, .app bundle)"
info "target dir: $TARGET_DIR — first run is a cold build, later runs are incremental"
export CARGO_TARGET_DIR="$TARGET_DIR"
# A dev install must never reach for the Developer ID cert or the notary key,
# even if this shell happens to have them exported from a release run.
unset APPLE_SIGNING_IDENTITY APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD \
      APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_PATH \
      APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID

pnpm tauri build --bundles app --config "$OVERLAY"

BUILT_APP="$TARGET_DIR/release/bundle/macos/$APP_NAME.app"
[[ -d "$BUILT_APP" ]] || die "expected bundle not found: $BUILT_APP"

# ─── 5. install into ~/Applications ─────────────────────────────────
step "Installing to $APPDIR"
# Replacing a bundle out from under a running app leaves it on unlinked inodes
# — it survives, but anything it lazy-loads afterwards is gone.
if pgrep -f "$APP_BUNDLE/Contents/MacOS/" >/dev/null 2>&1; then
  warn "$APP_NAME is running — quit it and rerun for a clean swap"
fi

mkdir -p "$APPDIR"
# Stage beside the destination, then swap, so a failed copy can't leave a
# half-written bundle where the old working one was.
STAGE="$APPDIR/.$APP_NAME.app.new"
rm -rf "$STAGE"
ditto "$BUILT_APP" "$STAGE"
rm -rf "$APP_BUNDLE"
mv "$STAGE" "$APP_BUNDLE"
# Nudge Launch Services so the Dock and Spotlight pick up the new copy.
touch "$APP_BUNDLE"

printf '%s\n' "$SHA" > "$STAMP"

step "Installed $APP_NAME $BUILD_VERSION"
info "commit    $SHORT — $SUBJECT"
info "path      $APP_BUNDLE"
info "launch    open -a \"$APP_NAME\"   (or \"$APP_NAME\" in Spotlight)"
info "data      $DATA_DIR  (separate from the released app)"
info "refresh   $REPO/scripts/dev-install-macos.sh"
