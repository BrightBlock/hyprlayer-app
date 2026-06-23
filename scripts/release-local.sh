#!/usr/bin/env bash
# Cut a macOS (Apple Silicon) + Windows (x64) release entirely from this machine.
# This is THE release path: GitHub Actions was removed to save minutes, so there
# is no CI release — everything below is published from here, end to end:
#   1. build + ed25519-sign the arm64 .dmg and updater .app.tar.gz (locally)
#   2. build the Windows x64 NSIS installer on a remote Windows builder over SSH
#      (nexus-win by default), copy it back, and ed25519-sign it HERE — so the
#      signing key never leaves this Mac and the builder needs no secrets
#   3. create the GitHub Release on the PUBLIC releases repo (binaries only —
#      never the source) and upload both platforms' assets
#   4. write the signed updater manifest (latest.json — darwin-aarch64 +
#      windows-x86_64) + changelog to brightblock.ai, so running apps update
#   5. bump the Homebrew cask (version + dmg sha256)
#
# The Windows installer is built from a clean `git archive` of the release tag
# shipped to the builder — NOT from any working clone there — so it matches what
# this Mac publishes byte for byte. The builder needs Rust (x86_64-pc-windows-msvc
# target), Node + pnpm, VS Build Tools, and `tar`; it does NOT need the signing
# key or Git credentials.
#
# Pass --mac-only to skip Windows entirely (the old behavior): the manifest then
# carries a single darwin-aarch64 key, so any Windows/Intel install just sees
# "no update" until the next all-platform release.
#
# Usage:
#   ./scripts/release.sh patch              # step 1: bump version, write changelog,
#                                           #         commit + tag (local, no push)
#   ./scripts/release-local.sh              # step 2: build + publish the cut tag
#   ./scripts/release-local.sh -y           # ...skipping the pre-publish confirmation
#   ./scripts/release-local.sh --mac-only   # macOS arm64 only (no Windows)
#
# Prerequisites:
#   * gh authenticated (`gh auth status`) with push access to the releases repo,
#     the tap, and the website repo
#   * the Tauri updater signing key at ~/.tauri/hyprlayer_updater.key
#     (override with TAURI_SIGNING_PRIVATE_KEY_PATH). If it's password-protected,
#     export TAURI_SIGNING_PRIVATE_KEY_PASSWORD before running.
#   * (unless --mac-only) SSH access to the Windows builder. Override the host
#     with HYPRLAYER_WIN_SSH_HOST (default: nexus-win).
set -euo pipefail

cd "$(dirname "$0")/.."

# ─── config (publish targets — formerly release.yml's env) ──────────
RELEASES_REPO="BrightBlock/hyprlayer-releases"
TAP_REPO="BrightBlock/homebrew-tap"
WEBSITE_REPO="jtsiros/brightblock.ai"
WEBSITE_MANIFEST_PATH="public/updates/latest.json"
WEBSITE_CHANGELOG_PATH="public/updates/changelog.json"
CASK_PATH="Casks/hyprlayer-desktop.rb"
SIGNING_KEY_PATH="${TAURI_SIGNING_PRIVATE_KEY_PATH:-$HOME/.tauri/hyprlayer_updater.key}"
# Remote Windows builder, reached over SSH (a Tailscale MagicDNS name by default).
WIN_SSH_HOST="${HYPRLAYER_WIN_SSH_HOST:-nexus-win}"
WIN_HELPER="scripts/release-win-remote.ps1"
# Where the copied-down + signed Windows installer lands (under target/, which is
# gitignored — mirrors where the mac bundle lives).
WIN_OUT="target/windows-release"
# Vercel only deploys commits whose author email is tied to a Vercel account, so
# the website/tap commits must be authored as the project owner (same as CI).
PUSH_AUTHOR_NAME="jtsiros"
PUSH_AUTHOR_EMAIL="jon@brightblock.ai"

# ─── args ───────────────────────────────────────────────────────────
ASSUME_YES=0
INCLUDE_WINDOWS=1
for arg in "$@"; do
  case "$arg" in
    -y|--yes)   ASSUME_YES=1 ;;
    --mac-only) INCLUDE_WINDOWS=0 ;;
    *) echo "unknown arg: $arg (expected -y/--yes or --mac-only)" >&2; exit 1 ;;
  esac
done

say() { printf '\033[1;36m→ %s\033[0m\n' "$*"; }
ok()  { printf '\033[1;32m✓ %s\033[0m\n' "$*"; }
die() { printf '\033[1;31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

# Temp clones/logs are removed on any exit.
TMPDIRS=()
cleanup() { for d in "${TMPDIRS[@]:-}"; do [[ -n "${d:-}" ]] && rm -rf "$d"; done; }
trap cleanup EXIT

# ─── preflight ──────────────────────────────────────────────────────
REQUIRED_TOOLS=(gh cargo pnpm shasum node rustup curl)
[[ "$INCLUDE_WINDOWS" == "1" ]] && REQUIRED_TOOLS+=(ssh scp git)
for tool in "${REQUIRED_TOOLS[@]}"; do
  command -v "$tool" >/dev/null || die "missing required tool: $tool"
done
gh auth status >/dev/null 2>&1 || die "gh not authenticated — run: gh auth login"
[[ -f "$SIGNING_KEY_PATH" ]] || die "signing key not found at $SIGNING_KEY_PATH (set TAURI_SIGNING_PRIVATE_KEY_PATH)"

BRANCH="$(git rev-parse --abbrev-ref HEAD)"
[[ "$BRANCH" == "main" ]] || die "releases are cut from main only (on $BRANCH)"

VERSION="$(node -p 'require("./package.json").version')"
TAG="v$VERSION"
git rev-parse "$TAG" >/dev/null 2>&1 || die "tag $TAG not found — run ./scripts/release.sh <bump> first"

# The changelog's top entry is what the manifest ships and the in-app card
# renders, so it must match the version being published.
CL_TOP="$(node -p 'JSON.parse(require("fs").readFileSync("changelog.json","utf8")).versions[0].version' 2>/dev/null || echo "")"
[[ "$CL_TOP" == "$VERSION" ]] || die "changelog.json top entry ($CL_TOP) != $VERSION — fix it and re-run ./scripts/release.sh"

# arm64 toolchain (no-op if already installed).
rustup target list --installed 2>/dev/null | grep -q '^aarch64-apple-darwin$' \
  || rustup target add aarch64-apple-darwin

# Windows builder reachability — fail now, before any build, with a clear path
# out (so we never silently publish a mac-only manifest when both were intended).
if [[ "$INCLUDE_WINDOWS" == "1" ]]; then
  [[ -f "$WIN_HELPER" ]] || die "missing $WIN_HELPER"
  say "Checking Windows builder ($WIN_SSH_HOST) is reachable…"
  ssh -o BatchMode=yes -o ConnectTimeout=10 "$WIN_SSH_HOST" "echo ok" >/dev/null 2>&1 \
    || die "Windows builder '$WIN_SSH_HOST' unreachable over SSH — wake it / check Tailscale, set HYPRLAYER_WIN_SSH_HOST, or run with --mac-only"
fi

PLATFORM_DESC="macOS arm64"
[[ "$INCLUDE_WINDOWS" == "1" ]] && PLATFORM_DESC="macOS arm64 + Windows x64"
say "Releasing Hyprlayer $TAG — $PLATFORM_DESC, locally"

TMP="$(mktemp -d)"; TMPDIRS+=("$TMP")

# ─── 1a. kick off the Windows build (overlaps the mac compile) ──────
# The remote NSIS build is the long pole (~15 min cold), so start it now in the
# background and collect the result after the mac build. It builds a clean
# `git archive` of $TAG — never the builder's working clone. The helper defaults
# its source/build paths under the builder's home dir, so we just scp there.
WIN_PID=""
WIN_LOG="$TMP/windows-build.log"
if [[ "$INCLUDE_WINDOWS" == "1" ]]; then
  say "Shipping $TAG source to $WIN_SSH_HOST and starting NSIS build (background)…"
  git archive --format=tar.gz -o "$TMP/hyprlayer-src.tgz" "$TAG"
  scp -q "$TMP/hyprlayer-src.tgz" "$WIN_SSH_HOST:hyprlayer-src.tgz"
  scp -q "$WIN_HELPER" "$WIN_SSH_HOST:release-win-remote.ps1"
  ssh "$WIN_SSH_HOST" \
    "powershell -NoProfile -ExecutionPolicy Bypass -File %USERPROFILE%\\release-win-remote.ps1" \
    >"$WIN_LOG" 2>&1 &
  WIN_PID=$!
fi

# ─── 1b. build + sign mac arm64 ─────────────────────────────────────
say "Building + signing arm64 bundle (compiles Rust — a few minutes)…"
TAURI_SIGNING_PRIVATE_KEY="$(cat "$SIGNING_KEY_PATH")" \
TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" \
  pnpm tauri build --target aarch64-apple-darwin

BUNDLE="target/aarch64-apple-darwin/release/bundle"
DMG="$(ls "$BUNDLE"/dmg/*.dmg 2>/dev/null | head -n1 || true)"
TARBALL="$(ls "$BUNDLE"/macos/*.app.tar.gz 2>/dev/null | head -n1 || true)"
[[ -f "${DMG:-}" && -f "${TARBALL:-}" && -f "${TARBALL:-}.sig" ]] \
  || die "expected build artifacts missing under $BUNDLE"
DMG_NAME="$(basename "$DMG")"
TARBALL_NAME="$(basename "$TARBALL")"
DMG_SHA="$(shasum -a 256 "$DMG" | awk '{print $1}')"
SIG="$(cat "$TARBALL.sig")"
ok "Built $DMG_NAME (sha256 $DMG_SHA)"

# ─── 1c. collect + sign the Windows installer ───────────────────────
# Wait for the remote build, copy the installer down, and ed25519-sign it HERE
# (the key never leaves this Mac). A Windows failure aborts before any push.
SETUP_NAME=""; WIN_SIG=""
if [[ "$INCLUDE_WINDOWS" == "1" ]]; then
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
  say "Signing $SETUP_NAME on the Mac (key stays local)…"
  TAURI_SIGNING_PRIVATE_KEY="$(cat "$SIGNING_KEY_PATH")" \
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" \
    pnpm exec tauri signer sign "$WIN_OUT/$SETUP_NAME" >/dev/null
  [[ -f "$WIN_OUT/$SETUP_NAME.sig" ]] || die "signing did not produce $SETUP_NAME.sig"
  WIN_SIG="$(cat "$WIN_OUT/$SETUP_NAME.sig")"
  WIN_SHA="$(shasum -a 256 "$WIN_OUT/$SETUP_NAME" | awk '{print $1}')"
  ok "Built + signed $SETUP_NAME (sha256 $WIN_SHA)"
fi

# ─── confirm before any outward push ────────────────────────────────
if [[ "$ASSUME_YES" -ne 1 ]]; then
  echo ""
  echo "About to publish $TAG:"
  echo "  • push commit + tag to origin/main"
  if [[ "$INCLUDE_WINDOWS" == "1" ]]; then
    echo "  • GitHub Release on $RELEASES_REPO (dmg + updater tarball + Windows setup.exe + sigs)"
    echo "  • updater manifest (darwin-aarch64 + windows-x86_64) → $WEBSITE_REPO"
  else
    echo "  • GitHub Release on $RELEASES_REPO (dmg + updater tarball + sig)"
    echo "  • updater manifest (darwin-aarch64 only) → $WEBSITE_REPO"
    echo "  Windows + Intel installs will NOT be served this version."
  fi
  echo "  • Homebrew cask bump → $TAP_REPO"
  read -r -p "Proceed? [y/N] " reply
  [[ "$reply" == "y" || "$reply" == "Y" ]] || die "aborted (build artifacts kept under target/)"
fi

# ─── 2. push source commit + tag ────────────────────────────────────
say "Pushing $TAG to origin/main…"
git push origin main
git push origin "$TAG"

# ─── 3. GitHub Release (binaries only, on the PUBLIC releases repo) ──
say "Creating GitHub Release on ${RELEASES_REPO}…"
# Release notes go to a temp file rather than a "$(cat <<EOF …)" capture: the
# macOS system bash (/bin/bash 3.2, what `env bash` resolves to here) can't
# parse a heredoc nested inside a double-quoted command substitution, so that
# form fails with "unexpected EOF". A plain heredoc + --notes-file is portable.
NOTES_FILE="$(mktemp)"; TMPDIRS+=("$NOTES_FILE")
cat > "$NOTES_FILE" <<EOF
## Install

### macOS (Apple Silicon)

Download \`$DMG_NAME\` below and drag it to /Applications. Apple Silicon only;
builds aren't notarized yet, so strip the quarantine flag once so Gatekeeper
lets it launch:

\`\`\`sh
xattr -dr com.apple.quarantine /Applications/Hyprlayer.app
\`\`\`
EOF
if [[ "$INCLUDE_WINDOWS" == "1" ]]; then
cat >> "$NOTES_FILE" <<EOF

### Windows (x64)

Download and run \`$SETUP_NAME\`. It isn't code-signed yet, so SmartScreen shows
an "unknown publisher" prompt — choose **More info → Run anyway**.
EOF
fi
cat >> "$NOTES_FILE" <<EOF

## Updates

After the first install the app self-updates via the in-app **Update** pill
(bottom-left of the sidebar).
EOF

# darwin assets always; windows assets when built.
ASSETS=("$DMG" "$TARBALL" "$TARBALL.sig")
if [[ "$INCLUDE_WINDOWS" == "1" ]]; then
  ASSETS+=("$WIN_OUT/$SETUP_NAME" "$WIN_OUT/$SETUP_NAME.sig")
fi
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
# Build the platforms map: darwin-aarch64 always; windows-x86_64 when built.
# A locally cut --mac-only release omits the windows key, so Windows installs
# just see "no update" until the next all-platform release.
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

echo ""
ok "Released Hyprlayer $TAG ($PLATFORM_DESC)"
echo "  Release : https://github.com/$RELEASES_REPO/releases/tag/$TAG"
echo "  Updates : https://brightblock.ai/hyprlayer/updates/latest.json"
if [[ "$INCLUDE_WINDOWS" != "1" ]]; then
  echo "  Note    : Windows + Intel installs stay on their current version until"
  echo "            the next all-platform release."
fi
