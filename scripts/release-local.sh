#!/usr/bin/env bash
# Cut a macOS (Apple Silicon) release entirely from this machine — the manual
# fallback for when GitHub Actions minutes are exhausted and
# .github/workflows/release.yml can't run.
#
# It reproduces the *macOS path* of that workflow, end to end:
#   1. build + ed25519-sign the arm64 .dmg and updater .app.tar.gz
#   2. create the GitHub Release on the PUBLIC releases repo (binaries only —
#      never the source) and upload the assets
#   3. write the signed updater manifest (latest.json) + changelog to
#      brightblock.ai, so running apps pick the update up
#   4. bump the Homebrew cask (version + dmg sha256)
#
# macOS arm64 ONLY. Windows and Intel Macs are NOT published here: the manifest
# this writes carries a single darwin-aarch64 key, so any Windows/Intel install
# just sees "no update" until the next full CI release. That's the price of the
# local fallback — for an all-platform release, push a tag and let CI run.
#
# Usage:
#   ./scripts/release.sh patch         # step 1: bump version, write changelog,
#                                      #         commit + tag (local, no push)
#   ./scripts/release-local.sh         # step 2: build + publish the cut tag
#   ./scripts/release-local.sh -y      # ...skipping the pre-publish confirmation
#
# Prerequisites:
#   * gh authenticated (`gh auth status`) with push access to the releases repo,
#     the tap, and the website repo
#   * the Tauri updater signing key at ~/.tauri/hyprlayer_updater.key
#     (override with TAURI_SIGNING_PRIVATE_KEY_PATH). If it's password-protected,
#     export TAURI_SIGNING_PRIVATE_KEY_PASSWORD before running.
set -euo pipefail

cd "$(dirname "$0")/.."

# ─── config (mirrors release.yml's env) ─────────────────────────────
RELEASES_REPO="BrightBlock/hyprlayer-releases"
TAP_REPO="BrightBlock/homebrew-tap"
WEBSITE_REPO="jtsiros/brightblock.ai"
WEBSITE_MANIFEST_PATH="public/updates/latest.json"
WEBSITE_CHANGELOG_PATH="public/updates/changelog.json"
CASK_PATH="Casks/hyprlayer-desktop.rb"
SIGNING_KEY_PATH="${TAURI_SIGNING_PRIVATE_KEY_PATH:-$HOME/.tauri/hyprlayer_updater.key}"
# Vercel only deploys commits whose author email is tied to a Vercel account, so
# the website/tap commits must be authored as the project owner (same as CI).
PUSH_AUTHOR_NAME="jtsiros"
PUSH_AUTHOR_EMAIL="jon@brightblock.ai"

ASSUME_YES=0
[[ "${1:-}" == "-y" || "${1:-}" == "--yes" ]] && ASSUME_YES=1

say() { printf '\033[1;36m→ %s\033[0m\n' "$*"; }
ok()  { printf '\033[1;32m✓ %s\033[0m\n' "$*"; }
die() { printf '\033[1;31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

# Temp clones are removed on any exit.
TMPDIRS=()
cleanup() { for d in "${TMPDIRS[@]:-}"; do [[ -n "${d:-}" ]] && rm -rf "$d"; done; }
trap cleanup EXIT

# ─── preflight ──────────────────────────────────────────────────────
for tool in gh cargo pnpm shasum node rustup curl; do
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

say "Releasing Hyprlayer $TAG — macOS arm64, locally"

# ─── 1. build + sign ────────────────────────────────────────────────
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

# ─── confirm before any outward push ────────────────────────────────
if [[ "$ASSUME_YES" -ne 1 ]]; then
  echo ""
  echo "About to publish $TAG:"
  echo "  • push commit + tag to origin/main"
  echo "  • GitHub Release on $RELEASES_REPO (dmg + updater tarball + sig)"
  echo "  • updater manifest (darwin-aarch64 only) → $WEBSITE_REPO"
  echo "  • Homebrew cask bump → $TAP_REPO"
  echo "  Windows + Intel installs will NOT be served this version."
  read -r -p "Proceed? [y/N] " reply
  [[ "$reply" == "y" || "$reply" == "Y" ]] || die "aborted (build artifacts kept under $BUNDLE)"
fi

# ─── 2. push source commit + tag ────────────────────────────────────
say "Pushing $TAG to origin/main…"
git push origin main
git push origin "$TAG"

# ─── 3. GitHub Release (binaries only, on the PUBLIC releases repo) ──
say "Creating GitHub Release on $RELEASES_REPO…"
NOTES="$(cat <<EOF
## Install

### macOS (Apple Silicon)

Download \`$DMG_NAME\` below and drag it to /Applications. Apple Silicon only;
builds aren't notarized yet, so strip the quarantine flag once so Gatekeeper
lets it launch:

\`\`\`sh
xattr -dr com.apple.quarantine /Applications/Hyprlayer.app
\`\`\`

## Updates

After the first install the app self-updates via the in-app **Update** pill
(bottom-left of the sidebar).
EOF
)"
if gh release view "$TAG" --repo "$RELEASES_REPO" >/dev/null 2>&1; then
  say "release $TAG already exists — re-uploading assets (--clobber)"
  gh release upload "$TAG" --repo "$RELEASES_REPO" --clobber \
    "$DMG" "$TARBALL" "$TARBALL.sig"
else
  gh release create "$TAG" --repo "$RELEASES_REPO" --target main \
    --title "$TAG" --notes "$NOTES" \
    "$DMG" "$TARBALL" "$TARBALL.sig"
fi
ok "Release published"

# ─── 4. updater manifest + changelog on brightblock.ai ──────────────
say "Writing updater manifest to $WEBSITE_REPO…"
SITE="$(mktemp -d)"; TMPDIRS+=("$SITE")
gh repo clone "$WEBSITE_REPO" "$SITE" -- --depth 1
BASE="https://github.com/$RELEASES_REPO/releases/download/$TAG"
mkdir -p "$SITE/$(dirname "$WEBSITE_MANIFEST_PATH")"
# Single darwin-aarch64 key — see the header note on why Windows/Intel are
# intentionally absent from a locally cut release.
cat > "$SITE/$WEBSITE_MANIFEST_PATH" <<JSON
{
  "version": "$VERSION",
  "notes": "See https://github.com/$RELEASES_REPO/releases/tag/$TAG",
  "pub_date": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "platforms": {
    "darwin-aarch64": {
      "signature": "$SIG",
      "url": "$BASE/$TARBALL_NAME"
    }
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
  git -C "$SITE" commit -q -m "hyprlayer: updater bump to $VERSION (arm64) + changelog"
  git -C "$SITE" push -q
fi
ok "Updater manifest live (Vercel redeploys within its cache TTL)"

# ─── 5. Homebrew cask bump ──────────────────────────────────────────
say "Bumping Homebrew cask in $TAP_REPO…"
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

echo ""
ok "Released Hyprlayer $TAG (macOS arm64)"
echo "  Release : https://github.com/$RELEASES_REPO/releases/tag/$TAG"
echo "  Updates : https://brightblock.ai/hyprlayer/updates/latest.json"
echo "  Note    : Windows + Intel installs stay on their current version until"
echo "            the next full CI release."
