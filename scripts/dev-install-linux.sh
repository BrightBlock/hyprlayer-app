#!/usr/bin/env bash
set -euo pipefail

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

CACHE_DIR="${HYPRLAYER_DEV_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/hyprlayer-dev}"
SRC_DIR="$CACHE_DIR/src"
TARGET_DIR="$CACHE_DIR/target"
STAMP="$CACHE_DIR/installed"
PREFIX="${HYPRLAYER_DEV_PREFIX:-$HOME/.local}"
APP_ID="ai.brightblock.hyprlayer.main"
APP_NAME="Hyprlayer Dev"
BIN_NAME="Hyprlayer-Dev"

INSTALLED_BIN="$PREFIX/bin/$BIN_NAME"
DESKTOP_FILE="$PREFIX/share/applications/$BIN_NAME.desktop"
DATA_DIR="$HOME/.local/share/$APP_ID"
CONFIG_DIR="$HOME/.config/$APP_ID"
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

usage() {
  printf '%s\n' \
    'dev-install-linux.sh - build and install "Hyprlayer Dev" from origin/main' \
    '' \
    '  --force               rebuild even if the ref has not moved' \
    '  --ref <ref>           track something else (a branch, tag, SHA)' \
    '  --no-fetch            use what is already fetched (offline)' \
    '  --no-version-suffix   build a bare x.y.z' \
    '  --uninstall           remove the install (keeps its data dir)'
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

step "Checking out $SHORT — $SUBJECT"
git -C "$SRC_DIR" checkout --quiet --detach "$SHA"
git -C "$SRC_DIR" reset --hard --quiet "$SHA"
git -C "$SRC_DIR" clean -qfd

cd "$SRC_DIR"

VERSION="$(node -p 'require("./package.json").version')"
if [[ $VERSION_SUFFIX -eq 1 ]]; then
  BUILD_VERSION="$VERSION+main.$SHORT"
else
  BUILD_VERSION="$VERSION"
fi

OVERLAY="src-tauri/tauri.devinstall.conf.json"
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

step "pnpm install (frozen lockfile)"
pnpm install --frozen-lockfile

step "Building $APP_NAME $BUILD_VERSION (release profile, no bundler)"
info "target dir: $TARGET_DIR — first run is a cold build, later runs are incremental"
export CARGO_TARGET_DIR="$TARGET_DIR"
pnpm tauri build --no-bundle --config "$OVERLAY"

BUILT_BIN="$TARGET_DIR/release/Hyprlayer"
[[ -x "$BUILT_BIN" ]] || die "expected binary not found: $BUILT_BIN"

step "Installing to $PREFIX"
mkdir -p "$PREFIX/bin"
install -m 755 "$BUILT_BIN" "$INSTALLED_BIN.new"
mv -f "$INSTALLED_BIN.new" "$INSTALLED_BIN"

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
