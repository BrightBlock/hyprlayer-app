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
# supervisor.rs spawns the server from beside the app binary, so the name is
# fixed and the two must land in the same directory.
INSTALLED_SERVER="$PREFIX/bin/hyprlayer-server"
DESKTOP_FILE="$PREFIX/share/applications/$BIN_NAME.desktop"
DATA_DIR="$HOME/.local/share/$APP_ID"
CONFIG_DIR="$HOME/.config/$APP_ID"
ICON_MAP=("32x32:32x32.png" "128x128:128x128.png" "256x256:128x128@2x.png")
INSTALL_PATHS=("$INSTALLED_BIN" "$INSTALLED_SERVER" "$DESKTOP_FILE")
for icon in "${ICON_MAP[@]}"; do
  INSTALL_PATHS+=("$PREFIX/share/icons/hicolor/${icon%%:*}/apps/$BIN_NAME.png")
done

REF="origin/main"
FORCE=0
FETCH=1
UNINSTALL=0
VERSION_SUFFIX=1
RESTART_WHEN_IDLE=0
REMOTES=()

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
    '  --remote <ssh-host>   also install this commit on a Linux host and share its' \
    '                        server (repeatable; the host must pin this profile)' \
    '  --restart-when-idle   stop the running server once nothing runs on it, so the' \
    '                        app reconnects to the new one (waits in the background)' \
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
    --restart-when-idle) RESTART_WHEN_IDLE=1; shift ;;
    --no-version-suffix) VERSION_SUFFIX=0; shift ;;
    --ref)       REF="${2:?--ref needs a value}"; shift 2 ;;
    --remote)    REMOTES+=("${2:?--remote needs a host}"); shift 2 ;;
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
  git -C "$SRC_DIR" fetch --quiet --prune --tags origin \
    || die "fetch failed (try --no-fetch to build what's already here)"
fi

SHA="$(git -C "$SRC_DIR" rev-parse --verify "$REF^{commit}" 2>/dev/null)" \
  || die "no such ref in the build clone: $REF"
SHORT="${SHA:0:8}"  # fixed width: --short varies by repo, and --remote compares versions across hosts
COMMITS="$(git -C "$SRC_DIR" rev-list --count "$SHA")"
SUBJECT="$(git -C "$SRC_DIR" log -1 --format=%s "$SHA")"

restart_when_idle() {
  [[ $RESTART_WHEN_IDLE -eq 1 ]] || return 0
  local log="$DATA_DIR/logs/restart-when-idle.log"
  mkdir -p "$(dirname "$log")"
  step "Stopping the $APP_ID server once it is idle"
  HYPRLAYER_APP_CONFIG_DIR="$CONFIG_DIR" HYPRLAYER_APP_DATA_DIR="$DATA_DIR" \
    HYPRLAYER_APP_LOG_DIR="$DATA_DIR/logs" \
    setsid -f nohup "$INSTALLED_SERVER" stop --when-idle >"$log" 2>&1 </dev/null
  info "waits for 2 minutes with no working agent or script; log in $log"
}

install_remotes() {
  [[ ${#REMOTES[@]} -gt 0 ]] || return 0
  local server_version flags=() host
  server_version="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$SRC_DIR/crates/hyprlayer-server/Cargo.toml" | head -n1)"
  [[ $FORCE -eq 1 ]] && flags+=(--force)
  [[ $VERSION_SUFFIX -eq 0 ]] && flags+=(--no-version-suffix)
  for host in "${REMOTES[@]}"; do
    "$SRC_DIR/scripts/dev-install-remote.sh" --host "$host" --ref "$SHA" --app-id "$APP_ID" \
      --version "$server_version" --cache-root "$DATA_DIR/server-cache" ${flags[@]+"${flags[@]}"} \
      || die "--remote $host failed"
  done
}

if [[ $FORCE -eq 0 && -x "$INSTALLED_BIN" && -f "$STAMP" ]] && grep -qx "$SHA" "$STAMP"; then
  step "Already on $REF ($SHORT — $SUBJECT)"
  info "Nothing to build. Use --force to rebuild anyway."
  restart_when_idle
  install_remotes
  exit 0
fi

step "Checking out $SHORT — $SUBJECT"
# --force: a prior build leaves its +dev version stamp in Cargo.toml/Cargo.lock.
git -C "$SRC_DIR" checkout --quiet --force --detach "$SHA"
git -C "$SRC_DIR" reset --hard --quiet "$SHA"
git -C "$SRC_DIR" clean -qfd

cd "$SRC_DIR"

# package.json only bumps in the release commit, so a build off untagged main would
# report the *previous* release and the updater would offer an upgrade to the one it
# already contains. Take the newest tag instead, and keep the sha as build metadata
# (`+`, not `-`) so semver reads it as equal to that release rather than older. The
# commit count before the sha is what lets the app order two dev builds.
VERSION="$(node -p 'require("./package.json").version')"
LATEST_TAG="$(git -C "$SRC_DIR" tag --list 'v[0-9]*' --sort=-v:refname | head -1)"
if [[ -n "$LATEST_TAG" ]]; then
  VERSION="$(printf '%s\n%s\n' "$VERSION" "${LATEST_TAG#v}" | sort -V | tail -1)"
fi
if [[ $VERSION_SUFFIX -eq 1 ]]; then
  BUILD_VERSION="$VERSION+dev.$COMMITS.$SHORT"
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

    // The app reports the workspace version; without this a dev build claims to
    // be the release it descends from.
    const manifest = fs.readFileSync("Cargo.toml", "utf8");
    // Exactly one, not at least one: `replace` without /g rewrites the *first*
    // match, so a second `version = "..."` at column 0 — a future dependency
    // block, say — would silently send this to the wrong line.
    const hits = manifest.match(/^version = ".*"$/gm) ?? [];
    if (hits.length !== 1) {
      throw new Error(
        `expected exactly one workspace version line in Cargo.toml, found ${hits.length}`,
      );
    }
    fs.writeFileSync(
      "Cargo.toml",
      manifest.replace(/^version = ".*"$/m, `version = "${process.env.BUILD_VERSION}"`),
    );

    // The server is versioned apart from the app, and its version keys the remote
    // install directory: without the same +dev suffix it collides with the release.
    const serverToml = "crates/hyprlayer-server/Cargo.toml";
    const server = fs.readFileSync(serverToml, "utf8");
    const serverHits = server.match(/^version = ".*"$/gm) ?? [];
    if (serverHits.length !== 1) {
      throw new Error("expected exactly one version line in " + serverToml + ", found " + serverHits.length);
    }
    const plus = process.env.BUILD_VERSION.indexOf("+");
    const suffix = plus === -1 ? "" : process.env.BUILD_VERSION.slice(plus);
    fs.writeFileSync(
      serverToml,
      server.replace(/^version = "([^"+]*)[^"]*"$/m, (_, base) => "version = \"" + base + suffix + "\""),
    );
  '

step "pnpm install (frozen lockfile)"
pnpm install --frozen-lockfile

step "Building $APP_NAME $BUILD_VERSION (release profile, no bundler)"
info "target dir: $TARGET_DIR — first run is a cold build, later runs are incremental"
export CARGO_TARGET_DIR="$TARGET_DIR"
pnpm tauri build --no-bundle --config "$OVERLAY"

BUILT_BIN="$TARGET_DIR/release/Hyprlayer"
[[ -x "$BUILT_BIN" ]] || die "expected binary not found: $BUILT_BIN"
# --no-bundle does no bundling, so nothing installs the sidecar; without this
# the app starts, finds no server beside it, and dies in the ENOENT dialog.
BUILT_SERVER="$TARGET_DIR/release/hyprlayer-server"
[[ -x "$BUILT_SERVER" ]] || die "expected server not found: $BUILT_SERVER — did beforeBuildCommand run?"

step "Installing to $PREFIX"
mkdir -p "$PREFIX/bin"
install -m 755 "$BUILT_BIN" "$INSTALLED_BIN.new"
mv -f "$INSTALLED_BIN.new" "$INSTALLED_BIN"
if [[ -e "$INSTALLED_SERVER" ]] && ! cmp -s "$BUILT_SERVER" "$INSTALLED_SERVER"; then
  info "replacing the existing $INSTALLED_SERVER"
fi
install -m 755 "$BUILT_SERVER" "$INSTALLED_SERVER.new"
mv -f "$INSTALLED_SERVER.new" "$INSTALLED_SERVER"

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

restart_when_idle
install_remotes
