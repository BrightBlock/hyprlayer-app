#!/usr/bin/env bash
# Cut a macOS (Apple Silicon) + Windows (x64) + Linux (x64) release entirely
# from this machine. This is THE release path: GitHub Actions was removed to save
# minutes, so there is no CI release — everything below is published from here,
# end to end:
#   1. build, Developer ID–sign + notarize, and ed25519-sign the arm64 .dmg and
#      updater .app.tar.gz (locally)
#   2. build the Windows x64 NSIS installer on a Windows builder — either nexus-win
#      over SSH (default) or a GitHub Actions windows-latest runner as a fallback
#      (HYPRLAYER_WIN_BUILDER=actions, see .github/workflows/windows-build.yml) —
#      copy it back, and ed25519-sign it HERE — so the signing key never leaves
#      this Mac and neither builder needs any secrets
#   3. build the Linux x64 .deb + .rpm on the nexus builder over SSH and copy
#      them back. No ed25519 signature: distro packages are owned by the package
#      manager, so the app never self-installs on Linux (see step 5).
#   4. create the GitHub Release on the PUBLIC releases repo (binaries only —
#      never the source), with changelog.json's "what's new" above the install
#      instructions, and upload every platform's assets
#   5. write the signed updater manifest (latest.json — darwin-aarch64 +
#      windows-x86_64) + changelog to brightblock.ai, so running apps update.
#      Linux is deliberately absent: those installs read the changelog and point
#      at pacman/apt/dnf instead of installing over themselves.
#   6. bump the Homebrew cask (version + dmg sha256)
#   7. push the rendered PKGBUILD/.SRCINFO to the AUR (hyprlayer-desktop-bin)
#
# Both remote installers are built from a clean `git archive` of the release tag
# shipped to the builder — NOT from any working clone there — so they match what
# this Mac publishes byte for byte. The Windows builder needs Rust
# (x86_64-pc-windows-msvc target), Node + pnpm, VS Build Tools, and `tar`; the
# Linux builder needs Rust, Node + pnpm, and the webkit2gtk-4.1 / gtk3 / librsvg
# / appindicator dev packages (see scripts/release-linux-remote.sh). Neither
# needs the signing key or Git credentials.
#
# Pass --mac-only to skip both remote builders (the old behavior): the manifest
# then carries a single darwin-aarch64 key, so any Windows/Linux/Intel install
# just sees "no update" until the next all-platform release. --no-windows /
# --no-linux drop one platform without dropping the other.
#
# Each platform's signed output for the current version is checked for on disk
# before that leg builds, so re-running after a transient failure (a flaky
# builder, a missing tool) reuses whatever already succeeded instead of
# rebuilding everything. Pass --rebuild to ignore the cache and force a clean
# build of every included platform.
#
# Usage:
#   ./scripts/release.sh patch              # step 1: bump version, write changelog,
#                                           #         commit + tag (local, no push)
#   ./scripts/release-local.sh              # step 2: build + publish the cut tag
#   ./scripts/release-local.sh -y           # ...skipping the pre-publish confirmation
#   ./scripts/release-local.sh --mac-only   # macOS arm64 only (no Windows, no Linux)
#   ./scripts/release-local.sh --no-linux   # macOS + Windows (skip deb/rpm + AUR)
#   ./scripts/release-local.sh --no-windows # macOS + Linux (skip the installer)
#   ./scripts/release-local.sh --no-aur     # build + publish Linux packages, but
#                                           # don't push the AUR package
#   ./scripts/release-local.sh --rebuild    # ignore cached artifacts, build fresh
#
# Prerequisites:
#   * gh authenticated (`gh auth status`) with push access to the releases repo,
#     the tap, and the website repo
#   * the Tauri updater signing key at ~/.tauri/hyprlayer_updater.key
#     (override with TAURI_SIGNING_PRIVATE_KEY_PATH). If it's password-protected,
#     export TAURI_SIGNING_PRIVATE_KEY_PASSWORD before running.
#   * the Developer ID Application cert in your login keychain + the App Store
#     Connect notary key (.p8) on disk, for macOS code signing + notarization.
#     Override the identity / key location via APPLE_SIGNING_IDENTITY and
#     APPLE_API_KEY_PATH (defaults are wired in the config block below).
#   * (unless --mac-only/--no-windows) SSH access to the Windows builder. Override
#     the host with HYPRLAYER_WIN_SSH_HOST (default: nexus-win). Or set
#     HYPRLAYER_WIN_BUILDER=actions to build on GitHub Actions instead (needs
#     no SSH — just `gh` auth with push access, already required above).
#   * (unless --mac-only/--no-linux) SSH access to the Linux builder. Override the
#     host with HYPRLAYER_LINUX_SSH_HOST (default: nexus).
#   * (unless --no-aur) the `aur-ci` key loaded in the 1Password SSH agent — the
#     same agent every other hyprlayer SSH host uses. Nothing AUR-specific to
#     set up; override the target with HYPRLAYER_AUR_SSH.
set -euo pipefail

cd "$(dirname "$0")/.."

# ─── config (publish targets — formerly release.yml's env) ──────────
RELEASES_REPO="BrightBlock/hyprlayer-releases"
TAP_REPO="BrightBlock/homebrew-tap"
WEBSITE_REPO="jtsiros/hyprlayer.dev"
WEBSITE_MANIFEST_PATH="public/updates/latest.json"
WEBSITE_CHANGELOG_PATH="public/updates/changelog.json"
CASK_PATH="Casks/hyprlayer-desktop.rb"
SIGNING_KEY_PATH="${TAURI_SIGNING_PRIVATE_KEY_PATH:-$HOME/.tauri/hyprlayer_updater.key}"
# Apple Developer ID signing + notarization (macOS). The .p8 notary key stays on
# disk like the updater key; only its non-secret identifiers live here. Tauri
# reads these from the env during `tauri build` (see step 1b). All env-overridable.
APPLE_SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:-Developer ID Application: JON TSIROS (V6KMPZJD7H)}"
APPLE_API_KEY="${APPLE_API_KEY:-JUMDZ7DQF5}"
APPLE_API_ISSUER="${APPLE_API_ISSUER:-4724564e-817f-4077-8b88-b4411b7978e9}"
APPLE_API_KEY_PATH="${APPLE_API_KEY_PATH:-$HOME/.tauri/AuthKey_JUMDZ7DQF5.p8}"
# Windows builder backend: "ssh" (nexus-win, default) or "actions" (GitHub
# Actions windows-latest fallback — see .github/workflows/windows-build.yml).
# Both are headless compile boxes only; signing/publishing always stay local.
WIN_BUILDER="${HYPRLAYER_WIN_BUILDER:-ssh}"
WIN_SSH_HOST="${HYPRLAYER_WIN_SSH_HOST:-nexus-win}"
WIN_HELPER="scripts/release-win-remote.ps1"
WIN_ACTIONS_REPO="${HYPRLAYER_WIN_ACTIONS_REPO:-BrightBlock/hyprlayer-app}"
WIN_ACTIONS_WORKFLOW="windows-build.yml"
# Linux builder: SSH only (nexus). Same contract as the Windows SSH path —
# headless compile box, no secrets; signing/publishing stay local.
LINUX_SSH_HOST="${HYPRLAYER_LINUX_SSH_HOST:-nexus}"
LINUX_HELPER="scripts/release-linux-remote.sh"
# AUR: the primary Linux channel. The package is binary-only (`-bin`) because the
# source repo is private; the builder renders PKGBUILD/.SRCINFO and this Mac
# pushes them. AUR only accepts `master`.
#
# Auth is the `aur-ci` key already in the 1Password SSH agent — the same agent
# `Host *` in ~/.ssh/config points every other hyprlayer host at, so there is no
# AUR-specific key to manage. Override HYPRLAYER_AUR_SSH with an ssh_config alias
# to pin a different identity.
AUR_PKGNAME="hyprlayer-desktop-bin"
AUR_SSH="${HYPRLAYER_AUR_SSH:-aur@aur.archlinux.org}"
AUR_REPO="ssh://${AUR_SSH}/${AUR_PKGNAME}.git"
AUR_AUTHOR_NAME="${HYPRLAYER_AUR_AUTHOR_NAME:-Jon Tsiros}"
AUR_AUTHOR_EMAIL="${HYPRLAYER_AUR_AUTHOR_EMAIL:-jon@brightblock.ai}"
# Where the copied-down + signed installers land (under target/, which is
# gitignored — mirrors where the mac bundle lives).
WIN_OUT="target/windows-release"
LINUX_OUT="target/linux-release"
# Vercel only deploys commits whose author email is tied to a Vercel account, so
# the website/tap commits must be authored as the project owner (same as CI).
PUSH_AUTHOR_NAME="jtsiros"
PUSH_AUTHOR_EMAIL="jon@brightblock.ai"

# ─── args ───────────────────────────────────────────────────────────
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
# The AUR package's source is the published .deb, so it can't be pushed without
# a Linux build.
[[ "$INCLUDE_LINUX" == "1" ]] || INCLUDE_AUR=0

say() { printf '\033[1;36m→ %s\033[0m\n' "$*"; }
ok()  { printf '\033[1;32m✓ %s\033[0m\n' "$*"; }
die() { printf '\033[1;31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

# Temp clones/logs are removed on any exit.
TMPDIRS=()
cleanup() { for d in "${TMPDIRS[@]:-}"; do [[ -n "${d:-}" ]] && rm -rf "$d"; done; }
trap cleanup EXIT

# ─── preflight ──────────────────────────────────────────────────────
REQUIRED_TOOLS=(gh cargo pnpm shasum node rustup curl)
# Either SSH builder needs the same three; the Actions path for Windows doesn't.
if [[ "$INCLUDE_LINUX" == "1" || ( "$INCLUDE_WINDOWS" == "1" && "$WIN_BUILDER" == "ssh" ) ]]; then
  REQUIRED_TOOLS+=(ssh scp git)
fi
for tool in "${REQUIRED_TOOLS[@]}"; do
  command -v "$tool" >/dev/null || die "missing required tool: $tool"
done
gh auth status >/dev/null 2>&1 || die "gh not authenticated — run: gh auth login"
[[ -f "$SIGNING_KEY_PATH" ]] || die "signing key not found at $SIGNING_KEY_PATH (set TAURI_SIGNING_PRIVATE_KEY_PATH)"
# Apple signing preflight — fail now, not 5 minutes into a build, if the cert or
# notary key is missing.
[[ -f "$APPLE_API_KEY_PATH" ]] || die "notarization key not found at $APPLE_API_KEY_PATH (set APPLE_API_KEY_PATH)"
security find-identity -v -p codesigning 2>/dev/null | grep -qF "$APPLE_SIGNING_IDENTITY" \
  || die "signing identity not in keychain: '$APPLE_SIGNING_IDENTITY' — import the Developer ID Application cert or set APPLE_SIGNING_IDENTITY"

BRANCH="$(git rev-parse --abbrev-ref HEAD)"
[[ "$BRANCH" == "main" ]] || die "releases are cut from main only (on $BRANCH)"

VERSION="$(node -p 'require("./package.json").version')"
TAG="v$VERSION"
git rev-parse "$TAG" >/dev/null 2>&1 || die "tag $TAG not found — run ./scripts/release.sh <bump> first"

# The changelog's top entry is what the manifest ships and the in-app card
# renders, so it must match the version being published.
CL_TOP="$(node -p 'JSON.parse(require("fs").readFileSync("changelog.json","utf8")).versions[0].version' 2>/dev/null || echo "")"
[[ "$CL_TOP" == "$VERSION" ]] || die "changelog.json top entry ($CL_TOP) != $VERSION — fix it and re-run ./scripts/release.sh"

# ─── artifact cache (resume support) ─────────────────────────────────
# A transient failure on one leg (flaky builder, missing tool) shouldn't force
# rebuilding legs that already succeeded. Each platform's signed output for
# this exact $VERSION is checked for on disk; --rebuild ignores all of this
# and forces a clean build. Windows/Linux filenames are already version-scoped
# so presence is enough; the mac updater tarball's name isn't (Hyprlayer.app.tar.gz,
# no version), so a marker file records which version it was last built for.
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

# arm64 toolchain (no-op if already installed).
rustup target list --installed 2>/dev/null | grep -q '^aarch64-apple-darwin$' \
  || rustup target add aarch64-apple-darwin

# Windows builder reachability — fail now, before any build, with a clear path
# out (so we never silently publish a mac-only manifest when both were intended).
# Skipped when the signed installer for this version is already cached.
if [[ "$INCLUDE_WINDOWS" == "1" && "$WIN_CACHED" != "1" ]]; then
  case "$WIN_BUILDER" in
    ssh)
      [[ -f "$WIN_HELPER" ]] || die "missing $WIN_HELPER"
      say "Checking Windows builder ($WIN_SSH_HOST) is reachable…"
      ssh -o BatchMode=yes -o ConnectTimeout=10 "$WIN_SSH_HOST" "echo ok" >/dev/null 2>&1 \
        || die "Windows builder '$WIN_SSH_HOST' unreachable over SSH — wake it / check Tailscale, set HYPRLAYER_WIN_SSH_HOST, run with HYPRLAYER_WIN_BUILDER=actions, or run with --mac-only"
      ;;
    actions)
      say "Checking GitHub Actions Windows builder ($WIN_ACTIONS_REPO) is reachable…"
      gh workflow view "$WIN_ACTIONS_WORKFLOW" --repo "$WIN_ACTIONS_REPO" >/dev/null 2>&1 \
        || die "workflow $WIN_ACTIONS_WORKFLOW not found on $WIN_ACTIONS_REPO — push .github/workflows/windows-build.yml to main first"
      ;;
    *) die "unknown HYPRLAYER_WIN_BUILDER '$WIN_BUILDER' (expected ssh or actions)" ;;
  esac
fi

# Same for the Linux builder — fail before any build rather than after the mac
# compile, so a sleeping nexus costs seconds instead of ten minutes. Skipped
# when the packages for this version are already cached.
if [[ "$INCLUDE_LINUX" == "1" && "$LINUX_CACHED" != "1" ]]; then
  [[ -f "$LINUX_HELPER" ]] || die "missing $LINUX_HELPER"
  [[ -f "packaging/aur/PKGBUILD.in" ]] || die "missing packaging/aur/PKGBUILD.in (the builder renders it)"
  say "Checking Linux builder ($LINUX_SSH_HOST) is reachable…"
  ssh -o BatchMode=yes -o ConnectTimeout=10 "$LINUX_SSH_HOST" "echo ok" >/dev/null 2>&1 \
    || die "Linux builder '$LINUX_SSH_HOST' unreachable over SSH — wake it / check Tailscale, set HYPRLAYER_LINUX_SSH_HOST, or run with --no-linux"
fi

# AUR auth too — a rejected push after the Release is already public leaves the
# AUR package a version behind, so prove the key works before anything ships.
# Same 1Password agent as the nexus check above, so a locked vault has already
# bitten by here; this only adds the aur-ci key to what it's asked to approve.
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

# ─── 1a. kick off the remote builds (they overlap the mac compile) ──
# Both remote builds are long poles (~15 min cold), so start them now in the
# background and collect the results after the mac build. Each builds a clean
# `git archive` of $TAG — never the builder's working clone. The helpers default
# their source/build paths under the builder's home dir, so we just scp there.
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
        # workflow_dispatch doesn't hand back a run id — poll for the run it just
        # created (most recent workflow_dispatch run at/after the trigger time).
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
  # `bash -lc` so a login shell resolves mise/nvm/volta/cargo shims — a bare
  # non-interactive SSH command often has none of them on PATH.
  ssh "$LINUX_SSH_HOST" "bash -lc 'bash ~/release-linux-remote.sh'" \
    >"$LINUX_LOG" 2>&1 &
  LINUX_PID=$!
fi

# ─── 1b. build + sign mac arm64 ─────────────────────────────────────
if [[ "$MAC_CACHED" == "1" ]]; then
  DMG="$MAC_DMG"
  TARBALL="$MAC_TARBALL"
else
  say "Building, signing + notarizing arm64 bundle (compiles Rust, then waits on Apple's notary — several minutes)…"
  # APPLE_* → Tauri Developer-ID-signs the .app, submits it to Apple's notary, and
  # staples the ticket (hardened runtime is enabled automatically). TAURI_SIGNING_*
  # is the separate ed25519 updater signature. Both run in this one build.
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

# ─── 1c. collect + sign the Windows installer ───────────────────────
# Wait for the remote build, copy the installer down, and ed25519-sign it HERE
# (the key never leaves this Mac). A Windows failure aborts before any push.
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
      # The helper prints `SETUP_EXE=<full windows path>` as its last line.
      SETUP_REMOTE="$(grep -E '^SETUP_EXE=' "$WIN_LOG" | tail -n1 | sed 's/^SETUP_EXE=//' | tr -d '\r')"
      [[ -n "$SETUP_REMOTE" ]] || { cat "$WIN_LOG"; die "no SETUP_EXE line in Windows build output"; }
      SETUP_REMOTE_FWD="${SETUP_REMOTE//\\//}"   # backslashes → slashes for scp + basename
      SETUP_NAME="$(basename "$SETUP_REMOTE_FWD")"
      mkdir -p "$WIN_OUT"
      scp -q "$WIN_SSH_HOST:${SETUP_REMOTE_FWD}" "$WIN_OUT/$SETUP_NAME" \
        || die "failed to copy $SETUP_NAME down from $WIN_SSH_HOST"
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
      SETUP_NAME="$(basename "$(ls "$WIN_OUT"/*-setup.exe | head -n1)")"
      [[ -n "$SETUP_NAME" ]] || die "no *-setup.exe found in downloaded artifact"
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

# ─── 1d. collect the Linux packages + the AUR files ─────────────────
# Unlike mac/Windows there is no ed25519 signature here: a distro package is
# installed and updated by pacman/apt/dnf, so the app never installs over
# itself on Linux and ships no updater payload. Integrity for the AUR path is
# the sha256 baked into the PKGBUILD the builder just rendered.
DEB_NAME=""; RPM_NAME=""
if [[ "$INCLUDE_LINUX" == "1" && "$LINUX_CACHED" == "1" ]]; then
  DEB_NAME="$LINUX_DEB_NAME"
  RPM_NAME="$(basename "$LINUX_RPM_EXPECTED")"
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
  # The helper prints DEB=/RPM=/AUR_DIR= as its last lines.
  linux_out_path() {
    grep -E "^$1=" "$LINUX_LOG" | tail -n1 | sed "s/^$1=//" | tr -d '\r'
  }
  DEB_REMOTE="$(linux_out_path DEB)"
  RPM_REMOTE="$(linux_out_path RPM)"
  AUR_REMOTE="$(linux_out_path AUR_DIR)"
  [[ -n "$DEB_REMOTE" && -n "$RPM_REMOTE" && -n "$AUR_REMOTE" ]] \
    || { cat "$LINUX_LOG"; die "Linux build output is missing a DEB/RPM/AUR_DIR line"; }
  DEB_NAME="$(basename "$DEB_REMOTE")"
  RPM_NAME="$(basename "$RPM_REMOTE")"
  mkdir -p "$LINUX_OUT"
  rm -rf "$LINUX_OUT/aur"
  scp -q "$LINUX_SSH_HOST:${DEB_REMOTE}" "$LINUX_OUT/$DEB_NAME" \
    || die "failed to copy $DEB_NAME down from $LINUX_SSH_HOST"
  scp -q "$LINUX_SSH_HOST:${RPM_REMOTE}" "$LINUX_OUT/$RPM_NAME" \
    || die "failed to copy $RPM_NAME down from $LINUX_SSH_HOST"
  # -r so the dotfile .SRCINFO comes along; a bare glob would drop it.
  scp -qr "$LINUX_SSH_HOST:${AUR_REMOTE}" "$LINUX_OUT/aur" \
    || die "failed to copy the AUR files down from $LINUX_SSH_HOST"
  [[ -f "$LINUX_OUT/aur/PKGBUILD" && -s "$LINUX_OUT/aur/.SRCINFO" ]] \
    || die "AUR files incomplete under $LINUX_OUT/aur (need PKGBUILD + .SRCINFO)"
  # The PKGBUILD pins the deb's sha256; if it doesn't match what we're about to
  # upload, every makepkg run fails validation.
  DEB_SHA="$(shasum -a 256 "$LINUX_OUT/$DEB_NAME" | awk '{print $1}')"
  grep -qF "$DEB_SHA" "$LINUX_OUT/aur/PKGBUILD" \
    || die "PKGBUILD sha256 does not match the copied-down $DEB_NAME ($DEB_SHA)"
  RPM_SHA="$(shasum -a 256 "$LINUX_OUT/$RPM_NAME" | awk '{print $1}')"
  ok "Built $DEB_NAME (sha256 $DEB_SHA)"
  ok "Built $RPM_NAME (sha256 $RPM_SHA)"
fi

# ─── confirm before any outward push ────────────────────────────────
if [[ "$ASSUME_YES" -ne 1 ]]; then
  echo ""
  # Name exactly the platform keys the manifest will carry, so a partial
  # release is never a surprise: anything missing here stays on its current
  # version until the next all-platform release.
  KEYS="darwin-aarch64"
  [[ "$INCLUDE_WINDOWS" == "1" ]] && KEYS="$KEYS + windows-x86_64"
  ASSET_DESC="dmg + updater tarball"
  [[ "$INCLUDE_WINDOWS" == "1" ]] && ASSET_DESC="$ASSET_DESC + Windows setup.exe"
  [[ "$INCLUDE_LINUX" == "1" ]] && ASSET_DESC="$ASSET_DESC + Linux deb/rpm"
  echo "About to publish $TAG:"
  echo "  • push commit + tag to origin/main"
  echo "  • GitHub Release on $RELEASES_REPO ($ASSET_DESC + sigs)"
  echo "  • updater manifest ($KEYS) → $WEBSITE_REPO"
  [[ "$INCLUDE_WINDOWS" == "1" ]] || echo "  Windows installs will NOT be served this version."
  echo "  • Homebrew cask bump → $TAP_REPO"
  [[ "$INCLUDE_AUR" == "1" ]] && echo "  • AUR push → $AUR_PKGNAME"
  [[ "$INCLUDE_LINUX" == "1" ]] || echo "  Linux packages will NOT be published this version."
  read -r -p "Proceed? [y/N] " reply
  [[ "$reply" == "y" || "$reply" == "Y" ]] || die "aborted (build artifacts kept under target/)"
fi

# ─── 2. push source commit + tag ────────────────────────────────────
say "Pushing $TAG to origin/main…"
git push origin main
git push origin "$TAG"

# ─── 3. GitHub Release (binaries only, on the PUBLIC releases repo) ──
say "Creating GitHub Release on ${RELEASES_REPO}…"
# darwin assets always; windows/linux assets when built. Built before the notes
# because the notes are rendered FROM this list — so the install block can only
# describe files that are actually being uploaded.
ASSETS=("$DMG" "$TARBALL" "$TARBALL.sig")
if [[ "$INCLUDE_WINDOWS" == "1" ]]; then
  ASSETS+=("$WIN_OUT/$SETUP_NAME" "$WIN_OUT/$SETUP_NAME.sig")
fi
if [[ "$INCLUDE_LINUX" == "1" ]]; then
  ASSETS+=("$LINUX_OUT/$DEB_NAME" "$LINUX_OUT/$RPM_NAME")
fi

# The body is "What's new" (from the same changelog.json entry the in-app Update
# card and the brightblock.ai changelog render, so the Release page can't drift
# from what the app tells people shipped) plus a compact install block that
# links to brightblock.ai for the rest. Step 0 already checked the changelog's
# top entry is $VERSION, and release.sh gates on it having a summary + notes, so
# a failure here means the changelog is wrong, not merely absent.
#
# Notes go to a temp file rather than a "$(cat <<EOF …)" capture: the macOS
# system bash (/bin/bash 3.2, what `env bash` resolves to here) can't parse a
# heredoc nested inside a double-quoted command substitution. --notes-file is
# portable, and a file also keeps the renderer's exit status meaningful.
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

# ─── 4. updater manifest + changelog on brightblock.ai ──────────────
say "Writing updater manifest to ${WEBSITE_REPO}…"
SITE="$(mktemp -d)"; TMPDIRS+=("$SITE")
gh repo clone "$WEBSITE_REPO" "$SITE" -- --depth 1
BASE="https://github.com/$RELEASES_REPO/releases/download/$TAG"
mkdir -p "$SITE/$(dirname "$WEBSITE_MANIFEST_PATH")"
# Build the platforms map: darwin-aarch64 always, windows-x86_64 when built. A
# locally cut --mac-only release omits Windows, so those installs just see "no
# update" until the next all-platform release.
#
# There is deliberately no linux-x86_64 key. Every Linux install now comes from a
# distro package that pacman/apt/dnf owns, and tauri-plugin-updater would try to
# install over it (dpkg -i under pkexec, or rewriting the binary in place) — on
# an AUR box that is simply wrong. Linux clients read changelog.json instead and
# point at the package manager; see src/lib/updater.ts.
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

# ─── 5. Homebrew cask bump ──────────────────────────────────────────
say "Bumping Homebrew cask in ${TAP_REPO}…"
TAP="$(mktemp -d)"; TMPDIRS+=("$TAP")
gh repo clone "$TAP_REPO" "$TAP" -- --depth 1
CASK="$TAP/$CASK_PATH"
[[ -f "$CASK" ]] || die "cask not found at $CASK_PATH in $TAP_REPO"
# Rewrite via a temp file (portable across BSD/GNU sed; only version + sha256
# change — the URL already templates the _aarch64.dmg name).
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

# ─── 6. verify the published assets resolve ─────────────────────────
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

# ─── 7. AUR package ─────────────────────────────────────────────────
# Last, because the PKGBUILD's source URL is the Release asset verified just
# above: pushing earlier would publish a package that can't download its source.
if [[ "$INCLUDE_AUR" == "1" ]]; then
  say "Pushing $AUR_PKGNAME $VERSION to the AUR…"
  AUR="$(mktemp -d)"; TMPDIRS+=("$AUR")
  git clone -q "$AUR_REPO" "$AUR" \
    || die "could not clone $AUR_REPO — check AUR SSH access, or re-run with --no-aur"
  # AUR rejects any branch but master, and a clone of a not-yet-created package
  # lands on whatever init.defaultBranch is set to locally.
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
fi

echo ""
ok "Released Hyprlayer $TAG ($PLATFORM_DESC)"
echo "  Release : https://github.com/$RELEASES_REPO/releases/tag/$TAG"
echo "  Updates : https://brightblock.ai/hyprlayer/updates/latest.json"
[[ "$INCLUDE_AUR" == "1" ]] && echo "  AUR     : https://aur.archlinux.org/packages/$AUR_PKGNAME"
if [[ "$INCLUDE_WINDOWS" != "1" ]]; then
  echo "  Note    : Windows + Intel installs stay on their current version until"
  echo "            the next all-platform release."
fi
if [[ "$INCLUDE_LINUX" != "1" ]]; then
  echo "  Note    : no Linux packages this version — the AUR/deb/rpm channels stay"
  echo "            on whatever was published last."
fi
