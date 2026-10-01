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
REMOTES=()

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
    '  --remote <ssh-host>   also install this commit on a Linux host and share its' \
    '                        server (repeatable; the host must pin this profile)' \
    '  --uninstall           remove the install (keeps its data dir)'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --force)     FORCE=1; shift ;;
    --no-fetch)  FETCH=0; shift ;;
    --uninstall) UNINSTALL=1; shift ;;
    --no-version-suffix) VERSION_SUFFIX=0; shift ;;
    --ref)       REF="${2:?--ref needs a value}"; shift 2 ;;
    --remote)    REMOTES+=("${2:?--remote needs a host}"); shift 2 ;;
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
  git -C "$SRC_DIR" fetch --quiet --prune --tags origin \
    || die "fetch failed (try --no-fetch to build what's already here)"
fi

SHA="$(git -C "$SRC_DIR" rev-parse --verify "$REF^{commit}" 2>/dev/null)" \
  || die "no such ref in the build clone: $REF"
SHORT="$(git -C "$SRC_DIR" rev-parse --short "$SHA")"
SUBJECT="$(git -C "$SRC_DIR" log -1 --format=%s "$SHA")"

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

if [[ $FORCE -eq 0 && -d "$APP_BUNDLE" && -f "$STAMP" ]] && grep -qx "$SHA" "$STAMP"; then
  step "Already on $REF ($SHORT — $SUBJECT)"
  info "Nothing to build. Use --force to rebuild anyway."
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
# (`+`, not `-`) so semver reads it as equal to that release rather than older.
VERSION="$(node -p 'require("./package.json").version')"
LATEST_TAG="$(git -C "$SRC_DIR" tag --list 'v[0-9]*' --sort=-v:refname | head -1)"
if [[ -n "$LATEST_TAG" ]]; then
  VERSION="$(printf '%s\n%s\n' "$VERSION" "${LATEST_TAG#v}" | sort -V | tail -1)"
fi
if [[ $VERSION_SUFFIX -eq 1 ]]; then
  BUILD_VERSION="$VERSION+dev.$SHORT"
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

install_remotes
