#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

RELEASES_REPO="BrightBlock/hyprlayer-releases"
TAP_REPO="BrightBlock/homebrew-tap"
WEBSITE_REPO="jtsiros/hyprlayer.dev"
WEBSITE_MANIFEST_PATH="public/updates/latest.json"
WEBSITE_CHANGELOG_PATH="public/updates/changelog.json"
CASK_PATH="Casks/hyprlayer-desktop.rb"
SIGNING_KEY_PATH="${TAURI_SIGNING_PRIVATE_KEY_PATH:-$HOME/.tauri/hyprlayer_updater.key}"
APPLE_SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:-Developer ID Application: JON TSIROS (V6KMPZJD7H)}"
APPLE_API_KEY="${APPLE_API_KEY:-JUMDZ7DQF5}"
APPLE_API_ISSUER="${APPLE_API_ISSUER:-4724564e-817f-4077-8b88-b4411b7978e9}"
APPLE_API_KEY_PATH="${APPLE_API_KEY_PATH:-$HOME/.tauri/AuthKey_JUMDZ7DQF5.p8}"
BUILDERS="scripts/builders/run.sh"
AUR_PKGNAME="hyprlayer-desktop-bin"
AUR_SERVER_PKGNAME="hyprlayer-server-bin"
AUR_SSH="${HYPRLAYER_AUR_SSH:-aur@aur.archlinux.org}"
AUR_REPO="ssh://${AUR_SSH}/${AUR_PKGNAME}.git"
AUR_SERVER_REPO="ssh://${AUR_SSH}/${AUR_SERVER_PKGNAME}.git"
FORMULA_PATH="Formula/hyprlayer-server.rb"
LAUNCHD_PLIST="packaging/launchd/ai.brightblock.hyprlayer.server.plist"
AUR_AUTHOR_NAME="${HYPRLAYER_AUR_AUTHOR_NAME:-Jon Tsiros}"
AUR_AUTHOR_EMAIL="${HYPRLAYER_AUR_AUTHOR_EMAIL:-jon@brightblock.ai}"
WIN_OUT="target/windows-release"
LINUX_OUT="target/linux-release"
PUSH_AUTHOR_NAME="jtsiros"
PUSH_AUTHOR_EMAIL="jon@brightblock.ai"

ASSUME_YES=0
INCLUDE_WINDOWS=1
INCLUDE_LINUX=1
INCLUDE_AUR=1
FORCE_REBUILD=0
SMOKE=0
for arg in "$@"; do
  case "$arg" in
    -y|--yes)     ASSUME_YES=1 ;;
    --mac-only)   INCLUDE_WINDOWS=0; INCLUDE_LINUX=0 ;;
    --no-windows) INCLUDE_WINDOWS=0 ;;
    --no-linux)   INCLUDE_LINUX=0 ;;
    --no-aur)     INCLUDE_AUR=0 ;;
    --rebuild)    FORCE_REBUILD=1 ;;
    --smoke)      SMOKE=1 ;;
    *) echo "unknown arg: $arg (expected -y/--yes, --mac-only, --no-windows, --no-linux, --no-aur, --rebuild or --smoke)" >&2; exit 1 ;;
  esac
done
[[ "$INCLUDE_LINUX" == "1" ]] || INCLUDE_AUR=0

say() { printf '\033[1;36m→ %s\033[0m\n' "$*"; }
ok()  { printf '\033[1;32m✓ %s\033[0m\n' "$*"; }
die() { printf '\033[1;31m✗ %s\033[0m\n' "$*" >&2; exit 1; }
warn() { printf '\033[1;33m! %s\033[0m\n' "$*" >&2; }

TMPDIRS=()
BUILDERS_PID=""
BUILDER_RUN="release-$$"
# Stops each process before listing its children, so none forks once it is seen;
# the pending TERM lands on CONT. Async jobs ignore Ctrl-C's SIGINT, hence TERM.
stop_builders() {
  [[ -n "$BUILDERS_PID" ]] || return 0
  say "Stopping the container builds…"
  local pids=("$BUILDERS_PID") i=0 p
  while (( i < ${#pids[@]} )); do
    p="${pids[i++]}"
    kill -STOP "$p" 2>/dev/null || continue
    pids+=($(pgrep -P "$p" || true))
  done
  kill -TERM "${pids[@]}" 2>/dev/null || true
  kill -CONT "${pids[@]}" 2>/dev/null || true
  local left
  for _ in {1..20}; do
    left="$(docker ps -aq --filter "label=hyprlayer-builder.run=$BUILDER_RUN" 2>/dev/null || true)"
    [[ -n "$left" ]] || return 0
    sleep 0.5
  done
  docker rm -f $left >/dev/null 2>&1 || true
}
cleanup() {
  stop_builders
  for d in "${TMPDIRS[@]:-}"; do [[ -n "${d:-}" ]] && rm -rf "$d"; done
}
trap cleanup EXIT

REQUIRED_TOOLS=(gh cargo pnpm shasum node rustup curl git)
[[ "$INCLUDE_LINUX" == "1" || "$INCLUDE_WINDOWS" == "1" ]] && REQUIRED_TOOLS+=(docker)
[[ "$INCLUDE_AUR" == "1" ]] && REQUIRED_TOOLS+=(ssh)
[[ "$SMOKE" == "1" && "$INCLUDE_WINDOWS" == "1" ]] && REQUIRED_TOOLS+=(ssh scp)
# The formula is rendered and syntax-checked after the release is already live.
[[ "$INCLUDE_LINUX" == "1" ]] && REQUIRED_TOOLS+=(python3 ruby)
for tool in "${REQUIRED_TOOLS[@]}"; do
  command -v "$tool" >/dev/null || die "missing required tool: $tool"
done
gh auth status >/dev/null 2>&1 || die "gh not authenticated — run: gh auth login"
[[ -f "$SIGNING_KEY_PATH" ]] || die "signing key not found at $SIGNING_KEY_PATH (set TAURI_SIGNING_PRIVATE_KEY_PATH)"
[[ -f "$APPLE_API_KEY_PATH" ]] || die "notarization key not found at $APPLE_API_KEY_PATH (set APPLE_API_KEY_PATH)"
security find-identity -v -p codesigning 2>/dev/null | grep -qF "$APPLE_SIGNING_IDENTITY" \
  || die "signing identity not in keychain: '$APPLE_SIGNING_IDENTITY' — import the Developer ID Application cert or set APPLE_SIGNING_IDENTITY"

BRANCH="$(git rev-parse --abbrev-ref HEAD)"
[[ "$BRANCH" == "main" ]] || die "releases are cut from main only (on $BRANCH)"

VERSION="$(node -p 'require("./package.json").version')"
TAG="v$VERSION"
git rev-parse "$TAG" >/dev/null 2>&1 || die "tag $TAG not found — run ./scripts/release.sh <bump> first"

# release.sh tags server-v<version> on the same commit as the app tag only when
# that server version is new, so this is stable across re-runs (ADR-0009).
SERVER_VERSION="$(sed -nE 's/^version = "(.*)"$/\1/p' crates/hyprlayer-server/Cargo.toml)"
[[ -n "$SERVER_VERSION" ]] || die "no version line in crates/hyprlayer-server/Cargo.toml"
SERVER_TAG="server-v$SERVER_VERSION"
git rev-parse "$SERVER_TAG" >/dev/null 2>&1 \
  || die "tag $SERVER_TAG not found — run ./scripts/release.sh <bump> first"
SHIP_SERVER=0
if [[ "$(git rev-list -n1 "$SERVER_TAG")" == "$(git rev-list -n1 "$TAG")" ]]; then
  SHIP_SERVER=1
  # A remote host downloads its own platform's binary from this release, and a
  # later release will not add to it.
  [[ "$INCLUDE_LINUX" == "1" && "$INCLUDE_WINDOWS" == "1" ]] \
    || die "hyprlayer-server $SERVER_VERSION is new, so this release must build every platform (drop --mac-only/--no-linux/--no-windows)"
fi

CL_TOP="$(node -p 'JSON.parse(require("fs").readFileSync("changelog.json","utf8")).versions[0].version' 2>/dev/null || echo "")"
[[ "$CL_TOP" == "$VERSION" ]] || die "changelog.json top entry ($CL_TOP) != $VERSION — fix it and re-run ./scripts/release.sh"

BUNDLE="target/aarch64-apple-darwin/release/bundle"
MAC_DMG="$BUNDLE/dmg/Hyprlayer_${VERSION}_aarch64.dmg"
MAC_TARBALL="$BUNDLE/macos/Hyprlayer.app.tar.gz"
MAC_MARKER="$BUNDLE/.release-version"
MAC_CACHED=0
if [[ "$FORCE_REBUILD" != "1" && -f "$MAC_DMG" && -f "$MAC_TARBALL" && -f "$MAC_TARBALL.sig" \
      && -f "$MAC_MARKER" && "$(cat "$MAC_MARKER" 2>/dev/null)" == "$VERSION" ]]; then
  MAC_CACHED=1
fi

WIN_SETUP_NAME="Hyprlayer_${VERSION}_x64-setup.exe"
WIN_CACHED=0
if [[ "$FORCE_REBUILD" != "1" && "$INCLUDE_WINDOWS" == "1" \
      && -f "$WIN_OUT/$WIN_SETUP_NAME" && -f "$WIN_OUT/$WIN_SETUP_NAME.sig" ]]; then
  WIN_CACHED=1
fi

LINUX_DEB_NAME="Hyprlayer_${VERSION}_amd64.deb"
LINUX_RPM_EXPECTED="$(ls "$LINUX_OUT"/Hyprlayer-"${VERSION}"-*.x86_64.rpm 2>/dev/null | head -n1 || true)"
LINUX_CACHED=0
if [[ "$FORCE_REBUILD" != "1" && "$INCLUDE_LINUX" == "1" \
      && -f "$LINUX_OUT/$LINUX_DEB_NAME" && -n "$LINUX_RPM_EXPECTED" && -f "$LINUX_RPM_EXPECTED" \
      && -f "$LINUX_OUT/aur/PKGBUILD" && -s "$LINUX_OUT/aur/.SRCINFO" ]] \
    && grep -qE "^pkgver=$VERSION\$" "$LINUX_OUT/aur/PKGBUILD" \
    && grep -qF "$(shasum -a 256 "$LINUX_OUT/$LINUX_DEB_NAME" | awk '{print $1}')" "$LINUX_OUT/aur/PKGBUILD"; then
  LINUX_CACHED=1
fi

[[ "$MAC_CACHED" == "1" ]] && say "Reusing already-built macOS bundle for $VERSION (pass --rebuild to force a fresh build)…"
[[ "$WIN_CACHED" == "1" ]] && say "Reusing already-built Windows installer for $VERSION (pass --rebuild to force a fresh build)…"
[[ "$LINUX_CACHED" == "1" ]] && say "Reusing already-built Linux packages for $VERSION (pass --rebuild to force a fresh build)…"

rustup target list --installed 2>/dev/null | grep -q '^aarch64-apple-darwin$' \
  || rustup target add aarch64-apple-darwin

BUILD_WINDOWS=0; BUILD_LINUX=0
[[ "$INCLUDE_WINDOWS" == "1" && "$WIN_CACHED" != "1" ]] && BUILD_WINDOWS=1
[[ "$INCLUDE_LINUX" == "1" && "$LINUX_CACHED" != "1" ]] && BUILD_LINUX=1
if [[ "$BUILD_WINDOWS" == "1" || "$BUILD_LINUX" == "1" || ( "$SMOKE" == "1" && "$INCLUDE_LINUX" == "1" ) ]]; then
  docker info >/dev/null 2>&1 \
    || die "Docker is not running — the Windows and Linux legs build in containers, and the Linux smoke test runs in them. Start Docker Desktop, or run with --mac-only"
fi
[[ "$BUILD_LINUX" != "1" || -f "packaging/aur/PKGBUILD.in" ]] \
  || die "missing packaging/aur/PKGBUILD.in (the builder renders it)"
if [[ "$SMOKE" == "1" && "$INCLUDE_WINDOWS" == "1" ]]; then
  say "Checking the Windows smoke-test host is up and idle…"
  scripts/builders/smoke/windows.sh --check \
    || die "the Windows smoke test cannot run (see above): start or free its host, or drop --smoke"
fi

if [[ "$INCLUDE_AUR" == "1" ]]; then
  say "Checking AUR SSH access (may prompt in 1Password)…"
  ssh -o BatchMode=yes -o ConnectTimeout=10 -o StrictHostKeyChecking=accept-new \
    "$AUR_SSH" help >/dev/null 2>&1 \
    || die "AUR SSH auth failed as '$AUR_SSH' — unlock 1Password and confirm the aur-ci key is loaded (\`ssh-add -l\`), or run with --no-aur"
fi

PLATFORM_DESC="macOS arm64"
[[ "$INCLUDE_WINDOWS" == "1" ]] && PLATFORM_DESC="$PLATFORM_DESC + Windows x64"
[[ "$INCLUDE_LINUX" == "1" ]] && PLATFORM_DESC="$PLATFORM_DESC + Linux x64"
say "Releasing Hyprlayer $TAG — $PLATFORM_DESC, locally"

TMP="$(mktemp -d)"; TMPDIRS+=("$TMP")

SRC_TGZ="$TMP/hyprlayer-src.tgz"
WIN_LOG="$TMP/windows-build.log"
LINUX_LOG="$TMP/linux-build.log"
build_leg() {  # build_leg <leg> <out-dir> <log>
  if HYPRLAYER_BUILDER_RUN="$BUILDER_RUN" "$BUILDERS" "$1" "$SRC_TGZ" "$2" >"$3" 2>&1; then echo 0; else echo $?; fi >"$TMP/$1.rc"
}
if [[ "$BUILD_WINDOWS" == "1" || "$BUILD_LINUX" == "1" ]]; then
  git archive --format=tar.gz -o "$SRC_TGZ" "$TAG"
  # Both legs at once peak near 10 GB. Docker Desktop's 8 GB setting reports a
  # MemTotal of 7.75 GiB and its 16 GB setting 15.59, so 11 GiB means 12 GB or more.
  DOCKER_MEM="$(docker info --format '{{.MemTotal}}' 2>/dev/null || true)"
  [[ "$DOCKER_MEM" =~ ^[0-9]+$ ]] || DOCKER_MEM=0
  PARALLEL=0
  (( DOCKER_MEM >= 11 * 1024 ** 3 )) && PARALLEL=1
  say "Building the Windows/Linux legs in containers from $TAG (background)…"
  if [[ "$BUILD_WINDOWS" == "1" && "$BUILD_LINUX" == "1" ]]; then
    if (( PARALLEL )); then
      say "  …both at once: Docker Desktop has $(( DOCKER_MEM / 1024 ** 3 )) GiB"
    else
      say "  …one after the other: Docker Desktop has under 12 GB"
    fi
  fi
  (
    if [[ "$BUILD_WINDOWS" == "1" ]]; then
      if (( PARALLEL )); then build_leg windows "$WIN_OUT" "$WIN_LOG" & else build_leg windows "$WIN_OUT" "$WIN_LOG"; fi
    fi
    [[ "$BUILD_LINUX" != "1" ]] || build_leg linux "$LINUX_OUT" "$LINUX_LOG"
    wait
  ) &
  BUILDERS_PID=$!
fi

wait_for_builders() {
  [[ -n "$BUILDERS_PID" ]] || return 0
  say "Waiting for the container builds…"
  wait "$BUILDERS_PID" || true
  BUILDERS_PID=""
}

builder_failed() {  # builder_failed <leg> <log>
  echo "── $1 build log ─────────────────────────────────────────"
  cat "$2"
  echo "──────────────────────────────────────────────────────────"
  die "$1 build failed in its container (log above)"
}

if [[ "$MAC_CACHED" == "1" ]]; then
  DMG="$MAC_DMG"
  TARBALL="$MAC_TARBALL"
else
  say "Building, signing + notarizing arm64 bundle (compiles Rust, then waits on Apple's notary — several minutes)…"
  APPLE_SIGNING_IDENTITY="$APPLE_SIGNING_IDENTITY" \
  APPLE_API_KEY="$APPLE_API_KEY" \
  APPLE_API_ISSUER="$APPLE_API_ISSUER" \
  APPLE_API_KEY_PATH="$APPLE_API_KEY_PATH" \
  TAURI_SIGNING_PRIVATE_KEY="$(cat "$SIGNING_KEY_PATH")" \
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" \
    pnpm tauri build --target aarch64-apple-darwin

  DMG="$(ls "$BUNDLE"/dmg/*.dmg 2>/dev/null | head -n1 || true)"
  TARBALL="$(ls "$BUNDLE"/macos/*.app.tar.gz 2>/dev/null | head -n1 || true)"
  [[ -f "${DMG:-}" && -f "${TARBALL:-}" && -f "${TARBALL:-}.sig" ]] \
    || die "expected build artifacts missing under $BUNDLE"
  echo "$VERSION" > "$MAC_MARKER"
fi
DMG_NAME="$(basename "$DMG")"
TARBALL_NAME="$(basename "$TARBALL")"
DMG_SHA="$(shasum -a 256 "$DMG" | awk '{print $1}')"
SIG="$(cat "$TARBALL.sig")"
if [[ "$MAC_CACHED" == "1" ]]; then
  ok "Reused $DMG_NAME (sha256 $DMG_SHA)"
else
  ok "Built $DMG_NAME (sha256 $DMG_SHA)"
fi
# The Developer ID-signed copy inside the notarized app, byte-identical to what
# the desktop runs. The staged sidecar under src-tauri/binaries/ is unsigned,
# and `tauri dev` restages a debug build over it.
MAC_SERVER_BIN="$TMP/hyprlayer-server-aarch64-apple-darwin"
cp "$BUNDLE/macos/Hyprlayer.app/Contents/MacOS/hyprlayer-server" "$MAC_SERVER_BIN" \
  || die "no hyprlayer-server inside $BUNDLE/macos/Hyprlayer.app — re-run with --rebuild"

SETUP_NAME=""; WIN_SIG=""
if [[ "$INCLUDE_WINDOWS" == "1" && "$WIN_CACHED" == "1" ]]; then
  SETUP_NAME="$WIN_SETUP_NAME"
  [[ -f "$WIN_OUT/hyprlayer-server-x86_64-pc-windows-msvc.exe" ]] \
    || die "cached Windows build has no hyprlayer-server-x86_64-pc-windows-msvc.exe — re-run with --rebuild"
  WIN_SIG="$(cat "$WIN_OUT/$SETUP_NAME.sig")"
  WIN_SHA="$(shasum -a 256 "$WIN_OUT/$SETUP_NAME" | awk '{print $1}')"
  ok "Reused $SETUP_NAME (sha256 $WIN_SHA)"
elif [[ "$INCLUDE_WINDOWS" == "1" ]]; then
  wait_for_builders
  [[ "$(cat "$TMP/windows.rc" 2>/dev/null)" == "0" ]] || builder_failed Windows "$WIN_LOG"
  SETUP_NAME="$WIN_SETUP_NAME"
  [[ -f "$WIN_OUT/$SETUP_NAME" && -f "$WIN_OUT/hyprlayer-server-x86_64-pc-windows-msvc.exe" ]] \
    || builder_failed Windows "$WIN_LOG"
  say "Signing $SETUP_NAME on the Mac (key stays local)…"
  TAURI_SIGNING_PRIVATE_KEY="$(cat "$SIGNING_KEY_PATH")" \
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" \
    pnpm exec tauri signer sign "$WIN_OUT/$SETUP_NAME" >/dev/null
  [[ -f "$WIN_OUT/$SETUP_NAME.sig" ]] || die "signing did not produce $SETUP_NAME.sig"
  WIN_SIG="$(cat "$WIN_OUT/$SETUP_NAME.sig")"
  WIN_SHA="$(shasum -a 256 "$WIN_OUT/$SETUP_NAME" | awk '{print $1}')"
  ok "Built + signed $SETUP_NAME (sha256 $WIN_SHA)"
fi

linux_out_path() {
  grep -E "^$1=" "$LINUX_LOG" | tail -n1 | sed "s/^$1=//" | tr -d '\r'
}

LINUX_SERVER_BIN="$LINUX_OUT/hyprlayer-server-x86_64-unknown-linux-gnu"
DEB_NAME=""; RPM_NAME=""
if [[ "$INCLUDE_LINUX" == "1" && "$LINUX_CACHED" == "1" ]]; then
  DEB_NAME="$LINUX_DEB_NAME"
  RPM_NAME="$(basename "$LINUX_RPM_EXPECTED")"
  [[ -f "$LINUX_SERVER_BIN" ]] \
    || die "cached Linux build has no $LINUX_SERVER_BIN — re-run with --rebuild"
  [[ -f "$LINUX_OUT/aur-server/PKGBUILD" && -s "$LINUX_OUT/aur-server/.SRCINFO" ]] \
    || die "cached Linux build has no server AUR files under $LINUX_OUT/aur-server — re-run with --rebuild"
  DEB_SHA="$(shasum -a 256 "$LINUX_OUT/$DEB_NAME" | awk '{print $1}')"
  RPM_SHA="$(shasum -a 256 "$LINUX_OUT/$RPM_NAME" | awk '{print $1}')"
  ok "Reused $DEB_NAME (sha256 $DEB_SHA)"
  ok "Reused $RPM_NAME (sha256 $RPM_SHA)"
elif [[ "$INCLUDE_LINUX" == "1" ]]; then
  wait_for_builders
  [[ "$(cat "$TMP/linux.rc" 2>/dev/null)" == "0" ]] || builder_failed Linux "$LINUX_LOG"
  DEB_NAME="$LINUX_DEB_NAME"
  RPM_NAME="$(basename "$(linux_out_path RPM)")"
  [[ -f "$LINUX_OUT/$DEB_NAME" && -f "$LINUX_OUT/$RPM_NAME" && -f "$LINUX_SERVER_BIN" ]] \
    || builder_failed Linux "$LINUX_LOG"
  [[ -f "$LINUX_OUT/aur/PKGBUILD" && -s "$LINUX_OUT/aur/.SRCINFO" ]] \
    || die "AUR files incomplete under $LINUX_OUT/aur (need PKGBUILD + .SRCINFO)"
  [[ -f "$LINUX_OUT/aur-server/PKGBUILD" && -s "$LINUX_OUT/aur-server/.SRCINFO" ]] \
    || die "server AUR files incomplete under $LINUX_OUT/aur-server (need PKGBUILD + .SRCINFO)"
  # $LINUX_OUT keeps past releases, so tie the PKGBUILDs to these exact bytes.
  SERVER_SHA_BUILT="$(linux_out_path SERVER_SHA)"
  grep -qF "$SERVER_SHA_BUILT" "$LINUX_OUT/aur-server/PKGBUILD" \
    || die "server PKGBUILD sha256 does not match the built server binary"
  [[ "$(shasum -a 256 "$LINUX_SERVER_BIN" | awk '{print $1}')" == "$SERVER_SHA_BUILT" ]] \
    || die "$LINUX_SERVER_BIN does not match the sha the builder reported"
  DEB_SHA="$(shasum -a 256 "$LINUX_OUT/$DEB_NAME" | awk '{print $1}')"
  grep -qF "$DEB_SHA" "$LINUX_OUT/aur/PKGBUILD" \
    || die "PKGBUILD sha256 does not match $DEB_NAME ($DEB_SHA)"
  RPM_SHA="$(shasum -a 256 "$LINUX_OUT/$RPM_NAME" | awk '{print $1}')"
  ok "Built $DEB_NAME (sha256 $DEB_SHA)"
  ok "Built $RPM_NAME (sha256 $RPM_SHA)"
fi

# Windows first: a stopped or busy test host fails in seconds, the Linux matrix takes minutes.
if [[ "$SMOKE" == "1" && "$INCLUDE_WINDOWS" == "1" ]]; then
  say "Smoke-testing $SETUP_NAME on the Windows test host…"
  scripts/builders/smoke/windows.sh "$WIN_OUT/$SETUP_NAME" "$WIN_OUT/hyprlayer-server-x86_64-pc-windows-msvc.exe" \
    || die "Windows smoke test failed, or its host was unreachable or busy (output above). Nothing was published: rerun once it passes, or without --smoke"
  ok "Windows smoke test passed"
fi
if [[ "$SMOKE" == "1" && "$INCLUDE_LINUX" == "1" ]]; then
  say "Smoke-testing the Linux packages on four distros (several minutes)…"
  scripts/builders/smoke/linux.sh "$LINUX_OUT" \
    || die "Linux smoke test failed (logs under target/smoke/linux/). Nothing was published: rerun once it passes, or without --smoke"
  ok "Linux smoke test passed"
fi

if [[ "$ASSUME_YES" -ne 1 ]]; then
  echo ""
  KEYS="darwin-aarch64"
  [[ "$INCLUDE_WINDOWS" == "1" ]] && KEYS="$KEYS + windows-x86_64"
  ASSET_DESC="dmg + updater tarball"
  [[ "$INCLUDE_WINDOWS" == "1" ]] && ASSET_DESC="$ASSET_DESC + Windows setup.exe"
  [[ "$INCLUDE_LINUX" == "1" ]] && ASSET_DESC="$ASSET_DESC + Linux deb/rpm"
  echo "About to publish $TAG:"
  echo "  • push commit + tag to origin/main"
  echo "  • GitHub Release on $RELEASES_REPO ($ASSET_DESC + sigs)"
  if [[ "$SHIP_SERVER" == "1" ]]; then
    echo "  • hyprlayer-server $SERVER_VERSION → $SERVER_TAG on $RELEASES_REPO"
  else
    echo "  • hyprlayer-server stays at $SERVER_VERSION (already released)"
  fi
  echo "  • updater manifest ($KEYS) → $WEBSITE_REPO"
  [[ "$INCLUDE_WINDOWS" == "1" ]] || echo "  Windows installs will NOT be served this version."
  echo "  • Homebrew cask bump → $TAP_REPO"
  [[ "$SHIP_SERVER" == "1" ]] && echo "  • Homebrew formula bump → $TAP_REPO ($FORMULA_PATH)"
  if [[ "$INCLUDE_AUR" == "1" && "$SHIP_SERVER" == "1" ]]; then
    echo "  • AUR push → $AUR_PKGNAME + $AUR_SERVER_PKGNAME"
  elif [[ "$INCLUDE_AUR" == "1" ]]; then
    echo "  • AUR push → $AUR_PKGNAME"
  fi
  [[ "$INCLUDE_LINUX" == "1" ]] || echo "  Linux packages will NOT be published this version."
  read -r -p "Proceed? [y/N] " reply
  [[ "$reply" == "y" || "$reply" == "Y" ]] || die "aborted (build artifacts kept under target/)"
fi

say "Pushing $TAG to origin/main…"
git push origin main
git push origin "$TAG"
[[ "$SHIP_SERVER" == "1" ]] && git push origin "$SERVER_TAG"

say "Creating GitHub Release on ${RELEASES_REPO}…"
ASSETS=("$DMG" "$TARBALL" "$TARBALL.sig")
if [[ "$INCLUDE_WINDOWS" == "1" ]]; then
  ASSETS+=("$WIN_OUT/$SETUP_NAME" "$WIN_OUT/$SETUP_NAME.sig")
fi
if [[ "$INCLUDE_LINUX" == "1" ]]; then
  ASSETS+=("$LINUX_OUT/$DEB_NAME" "$LINUX_OUT/$RPM_NAME")
fi

NOTES_FILE="$(mktemp)"; TMPDIRS+=("$NOTES_FILE")
HYPRLAYER_AUR_PKGNAME="$AUR_PKGNAME" \
  node scripts/release-notes.mjs "$VERSION" "${ASSETS[@]}" > "$NOTES_FILE" \
  || die "could not render release notes for $VERSION from changelog.json"

if gh release view "$TAG" --repo "$RELEASES_REPO" >/dev/null 2>&1; then
  say "release $TAG already exists — re-uploading assets (--clobber)"
  gh release upload "$TAG" --repo "$RELEASES_REPO" --clobber "${ASSETS[@]}"
else
  gh release create "$TAG" --repo "$RELEASES_REPO" --target main \
    --title "$TAG" --notes-file "$NOTES_FILE" "${ASSETS[@]}"
fi
ok "Release published"

SERVER_BASE="https://github.com/$RELEASES_REPO/releases/download/$SERVER_TAG"
if [[ "$SHIP_SERVER" == "1" ]]; then
  # Standalone server binaries, per target triple. These feed the hyprlayer-server
  # packages and the client-managed remote install (ADR-0009). Required, not
  # best-effort: a missing one publishes a release pointing at an absent asset.
  SERVER_ASSETS=("$MAC_SERVER_BIN" "$LINUX_SERVER_BIN" "$WIN_OUT/hyprlayer-server-x86_64-pc-windows-msvc.exe")
  for _s in "${SERVER_ASSETS[@]}"; do
    [[ -f "$_s" ]] || die "missing server asset $_s — every platform must ship one"
  done
  # Basename-keyed: a consumer downloads hyprlayer-server-<triple>, not a path.
  SERVER_SUMS="$TMP/hyprlayer-server-sha256sums.txt"
  : > "$SERVER_SUMS"
  for _s in "${SERVER_ASSETS[@]}"; do
    ( cd "$(dirname "$_s")" && shasum -a 256 "$(basename "$_s")" ) >> "$SERVER_SUMS"
  done
  say "Creating hyprlayer-server $SERVER_VERSION release on ${RELEASES_REPO}…"
  if gh release view "$SERVER_TAG" --repo "$RELEASES_REPO" >/dev/null 2>&1; then
    say "release $SERVER_TAG already exists — re-uploading assets (--clobber)"
    gh release upload "$SERVER_TAG" --repo "$RELEASES_REPO" --clobber "${SERVER_ASSETS[@]}" "$SERVER_SUMS"
  else
    gh release create "$SERVER_TAG" --repo "$RELEASES_REPO" --target main --latest=false \
      --title "hyprlayer-server $SERVER_VERSION" \
      --notes "hyprlayer-server $SERVER_VERSION, first shipped with Hyprlayer $TAG. Hyprlayer installs it on remote hosts itself; these binaries are for headless and packaged installs." \
      "${SERVER_ASSETS[@]}" "$SERVER_SUMS"
  fi
  ok "Server release published"
fi

say "Writing updater manifest to ${WEBSITE_REPO}…"
SITE="$(mktemp -d)"; TMPDIRS+=("$SITE")
gh repo clone "$WEBSITE_REPO" "$SITE" -- --depth 1
BASE="https://github.com/$RELEASES_REPO/releases/download/$TAG"
mkdir -p "$SITE/$(dirname "$WEBSITE_MANIFEST_PATH")"
PLATFORMS="    \"darwin-aarch64\": {
      \"signature\": \"$SIG\",
      \"url\": \"$BASE/$TARBALL_NAME\"
    }"
if [[ "$INCLUDE_WINDOWS" == "1" ]]; then
  PLATFORMS="$PLATFORMS,
    \"windows-x86_64\": {
      \"signature\": \"$WIN_SIG\",
      \"url\": \"$BASE/$SETUP_NAME\"
    }"
fi
cat > "$SITE/$WEBSITE_MANIFEST_PATH" <<JSON
{
  "version": "$VERSION",
  "notes": "See https://github.com/$RELEASES_REPO/releases/tag/$TAG",
  "pub_date": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "platforms": {
$PLATFORMS
  }
}
JSON
cp changelog.json "$SITE/$WEBSITE_CHANGELOG_PATH"
git -C "$SITE" config user.name "$PUSH_AUTHOR_NAME"
git -C "$SITE" config user.email "$PUSH_AUTHOR_EMAIL"
git -C "$SITE" add "$WEBSITE_MANIFEST_PATH" "$WEBSITE_CHANGELOG_PATH"
if git -C "$SITE" diff --cached --quiet; then
  say "manifest already current — nothing to commit"
else
  WIN_NOTE=""; [[ "$INCLUDE_WINDOWS" == "1" ]] && WIN_NOTE=" + win-x64"
  git -C "$SITE" commit -q -m "hyprlayer: updater bump to $VERSION (arm64${WIN_NOTE}) + changelog"
  git -C "$SITE" push -q
fi
ok "Updater manifest live (Vercel redeploys within its cache TTL)"

say "Bumping Homebrew cask in ${TAP_REPO}…"
TAP="$(mktemp -d)"; TMPDIRS+=("$TAP")
gh repo clone "$TAP_REPO" "$TAP" -- --depth 1
CASK="$TAP/$CASK_PATH"
[[ -f "$CASK" ]] || die "cask not found at $CASK_PATH in $TAP_REPO"
tmp="$(mktemp)"
sed -E -e "s|^  version \".*\"|  version \"$VERSION\"|" \
       -e "s|^  sha256 \".*\"|  sha256 \"$DMG_SHA\"|" \
       "$CASK" > "$tmp" && mv "$tmp" "$CASK"
grep -q "_aarch64.dmg" "$CASK" || die "cask URL is not _aarch64.dmg — fix the cask's url template first"
grep -qE "^  sha256 \"$DMG_SHA\"\$" "$CASK" || die "cask sha256 did not update as expected"
git -C "$TAP" config user.name "$PUSH_AUTHOR_NAME"
git -C "$TAP" config user.email "$PUSH_AUTHOR_EMAIL"
git -C "$TAP" add "$CASK_PATH"
if git -C "$TAP" diff --cached --quiet; then
  say "cask already at $VERSION — nothing to commit"
else
  git -C "$TAP" commit -q -m "hyprlayer-desktop $VERSION (arm64)"
  git -C "$TAP" push -q
fi
ok "Homebrew cask bumped"

# Only a new server version moves the formula; SHIP_SERVER already required
# every platform, so both sha256 values are fresh.
if [[ "$SHIP_SERVER" == "1" ]]; then
  say "Bumping hyprlayer-server formula in ${TAP_REPO}…"
  MAC_SERVER_SHA="$(shasum -a 256 "$MAC_SERVER_BIN" | awk '{print $1}')"
  LINUX_SERVER_SHA="$(shasum -a 256 "$LINUX_SERVER_BIN" | awk '{print $1}')"
  FORMULA_OUT="$TMP/hyprlayer-server.rb"
  # @PLIST@ takes the canonical launchd plist verbatim, indented into the
  # formula's squiggly heredoc, so the packaged and brewed plists cannot drift.
  VERSION="$SERVER_VERSION" MAC_SHA="$MAC_SERVER_SHA" LINUX_SHA="$LINUX_SERVER_SHA" \
  PLIST_SRC="$LAUNCHD_PLIST" TEMPLATE="packaging/homebrew/hyprlayer-server.rb" \
  python3 - "$FORMULA_OUT" <<'PYRENDER'
import os, sys
tpl = open(os.environ["TEMPLATE"]).read()
plist = open(os.environ["PLIST_SRC"]).read().rstrip("\n")
body = "\n".join("    " + line if line.strip() else "" for line in plist.split("\n"))
out = (tpl.replace("@VERSION@", os.environ["VERSION"])
          .replace("@SHA256_MACOS_ARM64@", os.environ["MAC_SHA"])
          .replace("@SHA256_LINUX_X86_64@", os.environ["LINUX_SHA"])
          .replace("@PLIST@", body))
# @BIN@ deliberately survives: the formula substitutes it at install time.
leftover = [m for m in ("@VERSION@", "@SHA256_MACOS_ARM64@",
                        "@SHA256_LINUX_X86_64@", "@PLIST@") if m in out]
if leftover:
    sys.exit(f"formula still has unsubstituted placeholders: {leftover}")
open(sys.argv[1], "w").write(out)
PYRENDER
  [[ -s "$FORMULA_OUT" ]] || die "rendering the hyprlayer-server formula produced nothing"
  ruby -c "$FORMULA_OUT" >/dev/null || die "the rendered formula is not valid Ruby"
  cp "$FORMULA_OUT" "$TAP/$FORMULA_PATH"
  git -C "$TAP" add "$FORMULA_PATH"
  if git -C "$TAP" diff --cached --quiet; then
    say "formula already at $SERVER_VERSION — nothing to commit"
  else
    git -C "$TAP" commit -q -m "hyprlayer-server $SERVER_VERSION"
    git -C "$TAP" push -q
  fi
  ok "Homebrew formula bumped"
else
  say "hyprlayer-server $SERVER_VERSION already released — formula left alone"
fi

say "Verifying published assets…"
curl -fsI -L "$BASE/$TARBALL_NAME" >/dev/null || die "tarball not reachable at $BASE/$TARBALL_NAME"
ok "updater tarball reachable"
curl -fsI -L "$BASE/$DMG_NAME" >/dev/null || die "dmg not reachable at $BASE/$DMG_NAME"
ok "dmg reachable"
if [[ "$INCLUDE_WINDOWS" == "1" ]]; then
  curl -fsI -L "$BASE/$SETUP_NAME" >/dev/null || die "setup.exe not reachable at $BASE/$SETUP_NAME"
  ok "Windows setup.exe reachable"
fi
if [[ "$INCLUDE_LINUX" == "1" ]]; then
  curl -fsI -L "$BASE/$DEB_NAME" >/dev/null || die "deb not reachable at $BASE/$DEB_NAME"
  ok "Linux .deb reachable"
  curl -fsI -L "$BASE/$RPM_NAME" >/dev/null || die "rpm not reachable at $BASE/$RPM_NAME"
  ok "Linux .rpm reachable"
fi
if [[ "$SHIP_SERVER" == "1" ]]; then
  curl -fsI -L "$SERVER_BASE/hyprlayer-server-sha256sums.txt" >/dev/null \
    || die "server checksums not reachable at $SERVER_BASE"
  ok "server release reachable"
fi

if [[ "$INCLUDE_AUR" == "1" ]]; then
  say "Pushing $AUR_PKGNAME $VERSION to the AUR…"
  AUR="$(mktemp -d)"; TMPDIRS+=("$AUR")
  git clone -q "$AUR_REPO" "$AUR" \
    || die "could not clone $AUR_REPO — check AUR SSH access, or re-run with --no-aur"
  git -C "$AUR" checkout -q -B master
  cp "$LINUX_OUT/aur/PKGBUILD" "$LINUX_OUT/aur/.SRCINFO" \
     "$LINUX_OUT/aur/hyprlayer-server.service" "$AUR/"
  git -C "$AUR" config user.name "$AUR_AUTHOR_NAME"
  git -C "$AUR" config user.email "$AUR_AUTHOR_EMAIL"
  git -C "$AUR" add PKGBUILD .SRCINFO hyprlayer-server.service
  if git -C "$AUR" diff --cached --quiet; then
    say "AUR package already at $VERSION — nothing to commit"
  else
    git -C "$AUR" commit -q -m "$AUR_PKGNAME $VERSION"
    git -C "$AUR" push -q origin master \
      || die "AUR push rejected — inspect $LINUX_OUT/aur and push by hand"
  fi
  ok "AUR package published"
fi

if [[ "$INCLUDE_AUR" == "1" && "$SHIP_SERVER" == "1" ]]; then
  say "Pushing $AUR_SERVER_PKGNAME $SERVER_VERSION to the AUR…"
  AUR_SERVER="$(mktemp -d)"; TMPDIRS+=("$AUR_SERVER")
  git clone -q "$AUR_SERVER_REPO" "$AUR_SERVER" \
    || die "could not clone $AUR_SERVER_REPO — check AUR SSH access, or re-run with --no-aur"
  git -C "$AUR_SERVER" checkout -q -B master
  # The unit is a local source= entry, so it is committed beside the PKGBUILD.
  cp "$LINUX_OUT/aur-server/PKGBUILD" "$LINUX_OUT/aur-server/.SRCINFO" \
     "$LINUX_OUT/aur-server/hyprlayer-server.service" "$AUR_SERVER/"
  git -C "$AUR_SERVER" config user.name "$AUR_AUTHOR_NAME"
  git -C "$AUR_SERVER" config user.email "$AUR_AUTHOR_EMAIL"
  git -C "$AUR_SERVER" add PKGBUILD .SRCINFO hyprlayer-server.service
  if git -C "$AUR_SERVER" diff --cached --quiet; then
    say "AUR server package already at $SERVER_VERSION — nothing to commit"
  else
    git -C "$AUR_SERVER" commit -q -m "$AUR_SERVER_PKGNAME $SERVER_VERSION"
    git -C "$AUR_SERVER" push -q origin master \
      || die "AUR push rejected — inspect $LINUX_OUT/aur-server and push by hand"
  fi
  ok "AUR server package published"
fi

echo ""
ok "Released Hyprlayer $TAG ($PLATFORM_DESC)"
echo "  Release : https://github.com/$RELEASES_REPO/releases/tag/$TAG"
echo "  Updates : https://brightblock.ai/hyprlayer/updates/latest.json"
[[ "$INCLUDE_AUR" == "1" ]] && echo "  AUR     : https://aur.archlinux.org/packages/$AUR_PKGNAME"
[[ "$SHIP_SERVER" == "1" ]] && echo "  Server  : https://github.com/$RELEASES_REPO/releases/tag/$SERVER_TAG"
[[ "$INCLUDE_AUR" == "1" && "$SHIP_SERVER" == "1" ]] && echo "  AUR srv : https://aur.archlinux.org/packages/$AUR_SERVER_PKGNAME"
if [[ "$INCLUDE_WINDOWS" != "1" ]]; then
  echo "  Note    : Windows + Intel installs stay on their current version until"
  echo "            the next all-platform release."
fi
if [[ "$INCLUDE_LINUX" != "1" ]]; then
  echo "  Note    : no Linux packages this version — the AUR/deb/rpm channels stay"
  echo "            on whatever was published last."
fi
