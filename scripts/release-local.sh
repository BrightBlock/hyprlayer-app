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
WIN_BUILDER="${HYPRLAYER_WIN_BUILDER:-ssh}"
WIN_SSH_HOST="${HYPRLAYER_WIN_SSH_HOST:-nexus-win}"
WIN_HELPER="scripts/release-win-remote.ps1"
WIN_ACTIONS_REPO="${HYPRLAYER_WIN_ACTIONS_REPO:-BrightBlock/hyprlayer-app}"
WIN_ACTIONS_WORKFLOW="windows-build.yml"
WIN_VM_HOST="${HYPRLAYER_WIN_VM_HOST:-nexus}"
WIN_VM_WAIT_TRIES="${HYPRLAYER_WIN_VM_WAIT_TRIES:-30}"
LINUX_SSH_HOST="${HYPRLAYER_LINUX_SSH_HOST:-nexus}"
LINUX_HELPER="scripts/release-linux-remote.sh"
AUR_PKGNAME="hyprlayer-desktop-bin"
AUR_SERVER_PKGNAME="hyprlayer-server-bin"
AUR_SSH="${HYPRLAYER_AUR_SSH:-aur@aur.archlinux.org}"
AUR_REPO="ssh://${AUR_SSH}/${AUR_PKGNAME}.git"
AUR_SERVER_REPO="ssh://${AUR_SSH}/${AUR_SERVER_PKGNAME}.git"
FORMULA_PATH="Formula/hyprlayer-server.rb"
MAC_SERVER_BIN="src-tauri/binaries/hyprlayer-server-aarch64-apple-darwin"
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
for arg in "$@"; do
  case "$arg" in
    -y|--yes)     ASSUME_YES=1 ;;
    --mac-only)   INCLUDE_WINDOWS=0; INCLUDE_LINUX=0 ;;
    --no-windows) INCLUDE_WINDOWS=0 ;;
    --no-linux)   INCLUDE_LINUX=0 ;;
    --no-aur)     INCLUDE_AUR=0 ;;
    --rebuild)    FORCE_REBUILD=1 ;;
    *) echo "unknown arg: $arg (expected -y/--yes, --mac-only, --no-windows, --no-linux, --no-aur or --rebuild)" >&2; exit 1 ;;
  esac
done
[[ "$INCLUDE_LINUX" == "1" ]] || INCLUDE_AUR=0

say() { printf '\033[1;36m→ %s\033[0m\n' "$*"; }
ok()  { printf '\033[1;32m✓ %s\033[0m\n' "$*"; }
die() { printf '\033[1;31m✗ %s\033[0m\n' "$*" >&2; exit 1; }
warn() { printf '\033[1;33m! %s\033[0m\n' "$*" >&2; }

TMPDIRS=()
cleanup() {
  for d in "${TMPDIRS[@]:-}"; do [[ -n "${d:-}" ]] && rm -rf "$d"; done
}
trap cleanup EXIT

REQUIRED_TOOLS=(gh cargo pnpm shasum node rustup curl)
if [[ "$INCLUDE_LINUX" == "1" || ( "$INCLUDE_WINDOWS" == "1" && "$WIN_BUILDER" == "ssh" ) ]]; then
  REQUIRED_TOOLS+=(ssh scp git)
fi
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

if [[ "$INCLUDE_WINDOWS" == "1" && "$WIN_CACHED" != "1" ]]; then
  case "$WIN_BUILDER" in
    ssh)
      [[ -f "$WIN_HELPER" ]] || die "missing $WIN_HELPER"
      # The guest's lifecycle belongs to omarchy-windows-vm, which owns both the
      # container name and a root-owned compose; bringing up a second compose
      # here would recreate the container under a different profile mid-release.
      say "Checking Windows builder ($WIN_SSH_HOST) is reachable…"
      win_ready=0
      for i in $(seq 1 "$WIN_VM_WAIT_TRIES"); do
        if ssh -o BatchMode=yes -o ConnectTimeout=10 "$WIN_SSH_HOST" "echo ok" >/dev/null 2>&1; then
          win_ready=1
          break
        fi
        [[ $((i % 6)) -eq 0 ]] && say "  …still waiting for the guest ($((i * 5))s)"
        sleep 5
      done
      [[ "$win_ready" == "1" ]] \
        || die "Windows builder '$WIN_SSH_HOST' unreachable over SSH — start the guest on $WIN_VM_HOST with 'omarchy-windows-vm launch', set HYPRLAYER_WIN_SSH_HOST, run with HYPRLAYER_WIN_BUILDER=actions, or run with --mac-only"
      ;;
    actions)
      say "Checking GitHub Actions Windows builder ($WIN_ACTIONS_REPO) is reachable…"
      gh workflow view "$WIN_ACTIONS_WORKFLOW" --repo "$WIN_ACTIONS_REPO" >/dev/null 2>&1 \
        || die "workflow $WIN_ACTIONS_WORKFLOW not found on $WIN_ACTIONS_REPO — push .github/workflows/windows-build.yml to main first"
      ;;
    *) die "unknown HYPRLAYER_WIN_BUILDER '$WIN_BUILDER' (expected ssh or actions)" ;;
  esac
fi

if [[ "$INCLUDE_LINUX" == "1" && "$LINUX_CACHED" != "1" ]]; then
  [[ -f "$LINUX_HELPER" ]] || die "missing $LINUX_HELPER"
  [[ -f "packaging/aur/PKGBUILD.in" ]] || die "missing packaging/aur/PKGBUILD.in (the builder renders it)"
  say "Checking Linux builder ($LINUX_SSH_HOST) is reachable…"
  ssh -o BatchMode=yes -o ConnectTimeout=10 "$LINUX_SSH_HOST" "echo ok" >/dev/null 2>&1 \
    || die "Linux builder '$LINUX_SSH_HOST' unreachable over SSH — wake it / check Tailscale, set HYPRLAYER_LINUX_SSH_HOST, or run with --no-linux"
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
if [[ ( "$INCLUDE_LINUX" == "1" && "$LINUX_CACHED" != "1" ) || ( "$INCLUDE_WINDOWS" == "1" && "$WIN_BUILDER" == "ssh" && "$WIN_CACHED" != "1" ) ]]; then
  git archive --format=tar.gz -o "$SRC_TGZ" "$TAG"
fi

WIN_PID=""
WIN_LOG="$TMP/windows-build.log"
if [[ "$INCLUDE_WINDOWS" == "1" && "$WIN_CACHED" != "1" ]]; then
  case "$WIN_BUILDER" in
    ssh)
      say "Shipping $TAG source to $WIN_SSH_HOST and starting NSIS build (background)…"
      scp -q "$SRC_TGZ" "$WIN_SSH_HOST:hyprlayer-src.tgz"
      scp -q "$WIN_HELPER" "$WIN_SSH_HOST:release-win-remote.ps1"
      ssh "$WIN_SSH_HOST" \
        "powershell -NoProfile -ExecutionPolicy Bypass -File %USERPROFILE%\\release-win-remote.ps1" \
        >"$WIN_LOG" 2>&1 &
      WIN_PID=$!
      ;;
    actions)
      say "Triggering GitHub Actions Windows build for $TAG (background)…"
      (
        set -euo pipefail
        TRIGGERED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        gh workflow run "$WIN_ACTIONS_WORKFLOW" --repo "$WIN_ACTIONS_REPO" -f "tag=$TAG"
        RUN_ID=""
        for _ in $(seq 1 15); do
          sleep 4
          RUN_ID="$(gh run list --repo "$WIN_ACTIONS_REPO" --workflow "$WIN_ACTIONS_WORKFLOW" \
            --limit 5 --json databaseId,createdAt,event \
            -q "[.[] | select(.event == \"workflow_dispatch\" and .createdAt >= \"$TRIGGERED_AT\")] | sort_by(.createdAt) | .[0].databaseId")"
          [[ -n "$RUN_ID" ]] && break
        done
        [[ -n "$RUN_ID" ]] || { echo "could not find the triggered Actions run"; exit 1; }
        echo "RUN_ID=$RUN_ID"
        gh run watch "$RUN_ID" --repo "$WIN_ACTIONS_REPO" --exit-status
      ) >"$WIN_LOG" 2>&1 &
      WIN_PID=$!
      ;;
  esac
fi

LINUX_PID=""
LINUX_LOG="$TMP/linux-build.log"
if [[ "$INCLUDE_LINUX" == "1" && "$LINUX_CACHED" != "1" ]]; then
  say "Shipping $TAG source to $LINUX_SSH_HOST and starting deb/rpm build (background)…"
  scp -q "$SRC_TGZ" "$LINUX_SSH_HOST:hyprlayer-src.tgz"
  scp -q "$LINUX_HELPER" "$LINUX_SSH_HOST:release-linux-remote.sh"
  ssh "$LINUX_SSH_HOST" "bash -lc 'bash ~/release-linux-remote.sh'" \
    >"$LINUX_LOG" 2>&1 &
  LINUX_PID=$!
fi

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

SETUP_NAME=""; WIN_SIG=""
if [[ "$INCLUDE_WINDOWS" == "1" && "$WIN_CACHED" == "1" ]]; then
  SETUP_NAME="$WIN_SETUP_NAME"
  WIN_SIG="$(cat "$WIN_OUT/$SETUP_NAME.sig")"
  WIN_SHA="$(shasum -a 256 "$WIN_OUT/$SETUP_NAME" | awk '{print $1}')"
  ok "Reused $SETUP_NAME (sha256 $WIN_SHA)"
elif [[ "$INCLUDE_WINDOWS" == "1" ]]; then
  case "$WIN_BUILDER" in
    ssh)
      say "Waiting for the Windows build on ${WIN_SSH_HOST}…"
      if ! wait "$WIN_PID"; then
        echo "── Windows build log ─────────────────────────────────────"
        cat "$WIN_LOG"
        echo "──────────────────────────────────────────────────────────"
        die "Windows build failed on $WIN_SSH_HOST (log above)"
      fi
      SETUP_REMOTE="$(grep -E '^SETUP_EXE=' "$WIN_LOG" | tail -n1 | sed 's/^SETUP_EXE=//' | tr -d '\r')"
      [[ -n "$SETUP_REMOTE" ]] || { cat "$WIN_LOG"; die "no SETUP_EXE line in Windows build output"; }
      SETUP_REMOTE_FWD="${SETUP_REMOTE//\\//}"
      SETUP_NAME="$(basename "$SETUP_REMOTE_FWD")"
      mkdir -p "$WIN_OUT"
      scp -q "$WIN_SSH_HOST:${SETUP_REMOTE_FWD}" "$WIN_OUT/$SETUP_NAME" \
        || die "failed to copy $SETUP_NAME down from $WIN_SSH_HOST"
      SERVER_REMOTE_WIN="$(grep -E '^SERVER_EXE=' "$WIN_LOG" | tail -n1 | sed 's/^SERVER_EXE=//' | tr -d '\r')"
      [[ -n "$SERVER_REMOTE_WIN" ]] \
        || { cat "$WIN_LOG"; die "no SERVER_EXE line in Windows build output"; }
      scp -q "$WIN_SSH_HOST:${SERVER_REMOTE_WIN//\\//}" \
        "$WIN_OUT/hyprlayer-server-x86_64-pc-windows-msvc.exe" \
        || die "failed to copy the Windows hyprlayer-server down from $WIN_SSH_HOST"
      ;;
    actions)
      say "Waiting for the GitHub Actions Windows build (${WIN_ACTIONS_REPO})…"
      if ! wait "$WIN_PID"; then
        echo "── Windows build log ─────────────────────────────────────"
        cat "$WIN_LOG"
        echo "──────────────────────────────────────────────────────────"
        die "Windows build failed on GitHub Actions (log above)"
      fi
      RUN_ID="$(grep -E '^RUN_ID=' "$WIN_LOG" | tail -n1 | sed 's/^RUN_ID=//' | tr -d '\r')"
      [[ -n "$RUN_ID" ]] || { cat "$WIN_LOG"; die "no RUN_ID line in Actions trigger output"; }
      mkdir -p "$WIN_OUT"
      gh run download "$RUN_ID" --repo "$WIN_ACTIONS_REPO" -n windows-installer -D "$WIN_OUT" \
        || die "failed to download windows-installer artifact from run $RUN_ID"
      # windows-build.yml uploads the installer only. The server asset needs the
      # ssh builder until that workflow uploads the sidecar too.
      [[ -f "$WIN_OUT/hyprlayer-server-x86_64-pc-windows-msvc.exe" ]] \
        || die "the Actions builder produces no hyprlayer-server asset — re-run with HYPRLAYER_WIN_BUILDER=ssh, or --no-windows"
      # $WIN_OUT keeps every past release's installer, so glob order would pick the oldest.
      SETUP_NAME="$WIN_SETUP_NAME"
      [[ -f "$WIN_OUT/$SETUP_NAME" ]] \
        || die "run $RUN_ID's artifact has no $SETUP_NAME — it built a different version than $TAG"
      ;;
  esac
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
  say "Waiting for the Linux build on ${LINUX_SSH_HOST}…"
  if ! wait "$LINUX_PID"; then
    echo "── Linux build log ───────────────────────────────────────"
    cat "$LINUX_LOG"
    echo "──────────────────────────────────────────────────────────"
    die "Linux build failed on $LINUX_SSH_HOST (log above)"
  fi
  DEB_REMOTE="$(linux_out_path DEB)"
  RPM_REMOTE="$(linux_out_path RPM)"
  AUR_REMOTE="$(linux_out_path AUR_DIR)"
  SERVER_REMOTE="$(linux_out_path SERVER_BIN)"
  AUR_SERVER_REMOTE="$(linux_out_path AUR_SERVER_DIR)"
  [[ -n "$DEB_REMOTE" && -n "$RPM_REMOTE" && -n "$AUR_REMOTE" \
     && -n "$SERVER_REMOTE" && -n "$AUR_SERVER_REMOTE" ]] \
    || { cat "$LINUX_LOG"; die "Linux build output is missing a DEB/RPM/AUR_DIR/SERVER_BIN/AUR_SERVER_DIR line"; }
  DEB_NAME="$(basename "$DEB_REMOTE")"
  RPM_NAME="$(basename "$RPM_REMOTE")"
  mkdir -p "$LINUX_OUT"
  rm -rf "$LINUX_OUT/aur"
  scp -q "$LINUX_SSH_HOST:${DEB_REMOTE}" "$LINUX_OUT/$DEB_NAME" \
    || die "failed to copy $DEB_NAME down from $LINUX_SSH_HOST"
  scp -q "$LINUX_SSH_HOST:${RPM_REMOTE}" "$LINUX_OUT/$RPM_NAME" \
    || die "failed to copy $RPM_NAME down from $LINUX_SSH_HOST"
  scp -qr "$LINUX_SSH_HOST:${AUR_REMOTE}" "$LINUX_OUT/aur" \
    || die "failed to copy the AUR files down from $LINUX_SSH_HOST"
  [[ -f "$LINUX_OUT/aur/PKGBUILD" && -s "$LINUX_OUT/aur/.SRCINFO" ]] \
    || die "AUR files incomplete under $LINUX_OUT/aur (need PKGBUILD + .SRCINFO)"
  scp -q "$LINUX_SSH_HOST:${SERVER_REMOTE}" "$LINUX_SERVER_BIN" \
    || die "failed to copy the Linux hyprlayer-server down from $LINUX_SSH_HOST"
  rm -rf "$LINUX_OUT/aur-server"
  scp -qr "$LINUX_SSH_HOST:${AUR_SERVER_REMOTE}" "$LINUX_OUT/aur-server" \
    || die "failed to copy the server AUR files down from $LINUX_SSH_HOST"
  [[ -f "$LINUX_OUT/aur-server/PKGBUILD" && -s "$LINUX_OUT/aur-server/.SRCINFO" ]] \
    || die "server AUR files incomplete under $LINUX_OUT/aur-server (need PKGBUILD + .SRCINFO)"
  SERVER_SHA_REMOTE="$(linux_out_path SERVER_SHA)"
  grep -qF "$SERVER_SHA_REMOTE" "$LINUX_OUT/aur-server/PKGBUILD" \
    || die "server PKGBUILD sha256 does not match the built server binary"
  [[ "$(shasum -a 256 "$LINUX_SERVER_BIN" | awk '{print $1}')" == "$SERVER_SHA_REMOTE" ]] \
    || die "the copied-down hyprlayer-server does not match the sha the builder reported"
  DEB_SHA="$(shasum -a 256 "$LINUX_OUT/$DEB_NAME" | awk '{print $1}')"
  grep -qF "$DEB_SHA" "$LINUX_OUT/aur/PKGBUILD" \
    || die "PKGBUILD sha256 does not match the copied-down $DEB_NAME ($DEB_SHA)"
  RPM_SHA="$(shasum -a 256 "$LINUX_OUT/$RPM_NAME" | awk '{print $1}')"
  ok "Built $DEB_NAME (sha256 $DEB_SHA)"
  ok "Built $RPM_NAME (sha256 $RPM_SHA)"
fi

if [[ "$ASSUME_YES" -ne 1 ]]; then
  echo ""
  KEYS="darwin-aarch64"
  [[ "$INCLUDE_WINDOWS" == "1" ]] && KEYS="$KEYS + windows-x86_64"
  ASSET_DESC="dmg + updater tarball + hyprlayer-server binaries"
  [[ "$INCLUDE_WINDOWS" == "1" ]] && ASSET_DESC="$ASSET_DESC + Windows setup.exe"
  [[ "$INCLUDE_LINUX" == "1" ]] && ASSET_DESC="$ASSET_DESC + Linux deb/rpm"
  echo "About to publish $TAG:"
  echo "  • push commit + tag to origin/main"
  echo "  • GitHub Release on $RELEASES_REPO ($ASSET_DESC + sigs)"
  echo "  • updater manifest ($KEYS) → $WEBSITE_REPO"
  [[ "$INCLUDE_WINDOWS" == "1" ]] || echo "  Windows installs will NOT be served this version."
  echo "  • Homebrew cask bump → $TAP_REPO"
  [[ "$INCLUDE_LINUX" == "1" ]] && echo "  • Homebrew formula bump → $TAP_REPO ($FORMULA_PATH)"
  [[ "$INCLUDE_AUR" == "1" ]] && echo "  • AUR push → $AUR_PKGNAME + $AUR_SERVER_PKGNAME"
  [[ "$INCLUDE_LINUX" == "1" ]] || echo "  Linux packages will NOT be published this version."
  read -r -p "Proceed? [y/N] " reply
  [[ "$reply" == "y" || "$reply" == "Y" ]] || die "aborted (build artifacts kept under target/)"
fi

say "Pushing $TAG to origin/main…"
git push origin main
git push origin "$TAG"

say "Creating GitHub Release on ${RELEASES_REPO}…"
ASSETS=("$DMG" "$TARBALL" "$TARBALL.sig")
if [[ "$INCLUDE_WINDOWS" == "1" ]]; then
  ASSETS+=("$WIN_OUT/$SETUP_NAME" "$WIN_OUT/$SETUP_NAME.sig")
fi
if [[ "$INCLUDE_LINUX" == "1" ]]; then
  ASSETS+=("$LINUX_OUT/$DEB_NAME" "$LINUX_OUT/$RPM_NAME")
fi

# Standalone server binaries, per target triple. These feed the hyprlayer-server
# packages and the client-managed remote install (ADR-0009). Required, not
# best-effort: a missing one publishes a release pointing at an absent asset.
SERVER_ASSETS=("$MAC_SERVER_BIN")
[[ "$INCLUDE_LINUX" == "1" ]] && SERVER_ASSETS+=("$LINUX_SERVER_BIN")
[[ "$INCLUDE_WINDOWS" == "1" ]] && SERVER_ASSETS+=("$WIN_OUT/hyprlayer-server-x86_64-pc-windows-msvc.exe")
for _s in "${SERVER_ASSETS[@]}"; do
  [[ -f "$_s" ]] || die "missing server asset $_s — every platform in this release must ship one"
done
# Basename-keyed: a consumer downloads hyprlayer-server-<triple>, not a path.
SERVER_SUMS="$TMP/hyprlayer-server-sha256sums.txt"
: > "$SERVER_SUMS"
for _s in "${SERVER_ASSETS[@]}"; do
  ( cd "$(dirname "$_s")" && shasum -a 256 "$(basename "$_s")" ) >> "$SERVER_SUMS"
done
ASSETS+=("${SERVER_ASSETS[@]}" "$SERVER_SUMS")

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

# The formula carries both platforms' sha256, so a mac-only release leaves the
# tap alone rather than publishing one with a stale Linux sha.
if [[ "$INCLUDE_LINUX" == "1" ]]; then
  say "Bumping hyprlayer-server formula in ${TAP_REPO}…"
  MAC_SERVER_SHA="$(shasum -a 256 "$MAC_SERVER_BIN" | awk '{print $1}')"
  LINUX_SERVER_SHA="$(shasum -a 256 "$LINUX_SERVER_BIN" | awk '{print $1}')"
  FORMULA_OUT="$TMP/hyprlayer-server.rb"
  # @PLIST@ takes the canonical launchd plist verbatim, indented into the
  # formula's squiggly heredoc, so the packaged and brewed plists cannot drift.
  VERSION="$VERSION" MAC_SHA="$MAC_SERVER_SHA" LINUX_SHA="$LINUX_SERVER_SHA" \
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
    say "formula already at $VERSION — nothing to commit"
  else
    git -C "$TAP" commit -q -m "hyprlayer-server $VERSION"
    git -C "$TAP" push -q
  fi
  ok "Homebrew formula bumped"
else
  warn "no Linux build this release — hyprlayer-server formula left at its previous version"
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

if [[ "$INCLUDE_AUR" == "1" ]]; then
  say "Pushing $AUR_PKGNAME $VERSION to the AUR…"
  AUR="$(mktemp -d)"; TMPDIRS+=("$AUR")
  git clone -q "$AUR_REPO" "$AUR" \
    || die "could not clone $AUR_REPO — check AUR SSH access, or re-run with --no-aur"
  git -C "$AUR" checkout -q -B master
  cp "$LINUX_OUT/aur/PKGBUILD" "$LINUX_OUT/aur/.SRCINFO" "$AUR/"
  git -C "$AUR" config user.name "$AUR_AUTHOR_NAME"
  git -C "$AUR" config user.email "$AUR_AUTHOR_EMAIL"
  git -C "$AUR" add PKGBUILD .SRCINFO
  if git -C "$AUR" diff --cached --quiet; then
    say "AUR package already at $VERSION — nothing to commit"
  else
    git -C "$AUR" commit -q -m "$AUR_PKGNAME $VERSION"
    git -C "$AUR" push -q origin master \
      || die "AUR push rejected — inspect $LINUX_OUT/aur and push by hand"
  fi
  ok "AUR package published"

  say "Pushing $AUR_SERVER_PKGNAME $VERSION to the AUR…"
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
    say "AUR server package already at $VERSION — nothing to commit"
  else
    git -C "$AUR_SERVER" commit -q -m "$AUR_SERVER_PKGNAME $VERSION"
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
[[ "$INCLUDE_AUR" == "1" ]] && echo "  AUR srv : https://aur.archlinux.org/packages/$AUR_SERVER_PKGNAME"
if [[ "$INCLUDE_WINDOWS" != "1" ]]; then
  echo "  Note    : Windows + Intel installs stay on their current version until"
  echo "            the next all-platform release."
fi
if [[ "$INCLUDE_LINUX" != "1" ]]; then
  echo "  Note    : no Linux packages this version — the AUR/deb/rpm channels stay"
  echo "            on whatever was published last."
fi
