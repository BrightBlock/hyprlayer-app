#!/usr/bin/env bash
set -euo pipefail

SELF="$0"
ORIGIN_SLUG="BrightBlock/hyprlayer-app"
BOOTSTRAP="bash <(gh api repos/$ORIGIN_SLUG/contents/scripts/dev-install-macos.sh -H 'Accept: application/vnd.github.raw')"
REPO=""
if self_dir="$(cd "$(dirname -- "$SELF")/.." 2>/dev/null && pwd)" \
  && [[ -e "$self_dir/.git" && -f "$self_dir/scripts/$(basename -- "$SELF")" ]]; then
  REPO="$self_dir"
fi

[[ "$(uname -s)" == Darwin ]] || {
  printf 'error: this is the macOS installer; on Linux use scripts/dev-install-linux.sh\n' >&2
  exit 1
}

CACHE_DIR="${HYPRLAYER_DEV_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/hyprlayer-dev}"
SRC_DIR="$CACHE_DIR/src"
TARGET_DIR="$CACHE_DIR/target"
STAMP="$CACHE_DIR/installed"
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

usage() {
  printf '%s\n' \
    'dev-install-macos.sh - build and install "Hyprlayer Dev" from origin/main' \
    '' \
    '  --force               rebuild even if the ref has not moved' \
    '  --ref <ref>           track something else (a branch, tag, SHA)' \
    '  --no-fetch            use what is already fetched (offline)' \
    '  --no-version-suffix   build a bare x.y.z' \
    '  --uninstall           remove the install (keeps its data dir)'
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
  rm -rfv "$APP_BUNDLE" "$STAMP"
  info "Kept (delete by hand if you want a clean slate):"
  info "  build cache  $CACHE_DIR"
  info "  app data     $DATA_DIR"
  exit 0
fi

for tool in git cargo pnpm node; do
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

if [[ $FORCE -eq 0 && -d "$APP_BUNDLE" && -f "$STAMP" ]] && grep -qx "$SHA" "$STAMP"; then
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

step "pnpm install (frozen lockfile)"
pnpm install --frozen-lockfile

step "Building $APP_NAME $BUILD_VERSION (release profile, .app bundle)"
info "target dir: $TARGET_DIR — first run is a cold build, later runs are incremental"
export CARGO_TARGET_DIR="$TARGET_DIR"
unset APPLE_SIGNING_IDENTITY APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD \
      APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_PATH \
      APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID

pnpm tauri build --bundles app --config "$OVERLAY"

BUILT_APP="$TARGET_DIR/release/bundle/macos/$APP_NAME.app"
[[ -d "$BUILT_APP" ]] || die "expected bundle not found: $BUILT_APP"

step "Installing to $APPDIR"
if pgrep -f "$APP_BUNDLE/Contents/MacOS/" >/dev/null 2>&1; then
  warn "$APP_NAME is running — quit it and rerun for a clean swap"
fi

mkdir -p "$APPDIR"
STAGE="$APPDIR/.$APP_NAME.app.new"
rm -rf "$STAGE"
ditto "$BUILT_APP" "$STAGE"
rm -rf "$APP_BUNDLE"
mv "$STAGE" "$APP_BUNDLE"
touch "$APP_BUNDLE"

printf '%s\n' "$SHA" > "$STAMP"

step "Installed $APP_NAME $BUILD_VERSION"
info "commit    $SHORT — $SUBJECT"
info "path      $APP_BUNDLE"
info "launch    open -a \"$APP_NAME\"   (or \"$APP_NAME\" in Spotlight)"
info "data      $DATA_DIR  (separate from the released app)"
if [[ -n "$REPO" ]]; then
  info "refresh   $REPO/scripts/dev-install-macos.sh"
else
  info "refresh   $BOOTSTRAP"
fi
