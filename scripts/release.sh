#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
BUMP="${1:-patch}"

if [[ -n "$(git status --porcelain | grep -v ' changelog\.json$' || true)" ]]; then
  echo "✗ working tree dirty (other than changelog.json) — commit or stash first"
  exit 1
fi
BRANCH="$(git rev-parse --abbrev-ref HEAD)"
if [[ "$BRANCH" != "main" ]]; then
  echo "✗ releases are cut from main only (currently on $BRANCH)"
  exit 1
fi

if [[ "${RELEASE_SKIP_CHECKS:-}" != "1" ]]; then
  echo "→ Verification gate (cargo test with local E2E, container E2E, library targets, typecheck, lint, vitest)…"
  # The container E2E tier reaches a Linux host over real SSH, the path remote
  # users take; check for Docker before the long steps rather than after.
  if ! docker info >/dev/null 2>&1; then
    echo "✗ Docker is not running — the E2E container tier needs it. Start Docker Desktop and re-run."
    exit 1
  fi
  # tauri-build refuses to compile the app without the externalBin sidecar, and
  # src-tauri/binaries/ is gitignored, so a clean checkout has none.
  node scripts/build-server-sidecar.mjs --debug
  cargo test --workspace --quiet
  cargo test --package hyprlayer-e2e --quiet -- --ignored
  # The desktop's own ssh sign-in path: it needs the app binary as its askpass helper.
  cargo test --package hyprlayer-app --test ssh_sign_in --quiet -- --ignored
  cargo check -p hyprlayer-models -p hyprlayer-core -p hyprlayer-proto   # specta-less: the app enables it, embedders don't
  pnpm typecheck && pnpm lint && pnpm test
  if [[ -n "$(git status --porcelain -- src/lib/bindings.ts)" ]]; then
    echo "✗ src/lib/bindings.ts drifted — cargo test regenerated it. Commit the diff first."
    exit 1
  fi
else
  echo "⚠ RELEASE_SKIP_CHECKS=1 — skipping the verification gate"
fi

# The server is versioned apart from the app (ADR-0009): a release ships a new
# one only when crates/hyprlayer-server's version has no server-v tag yet, and
# any server-side change since the last one must come with that bump.
SERVER_VERSION="$(sed -nE 's/^version = "(.*)"$/\1/p' crates/hyprlayer-server/Cargo.toml)"
[[ -n "$SERVER_VERSION" ]] || { echo "✗ no version line in crates/hyprlayer-server/Cargo.toml"; exit 1; }
SERVER_TAG="server-v$SERVER_VERSION"
SERVER_PATHS=(crates/hyprlayer-server crates/hyprlayer-core crates/hyprlayer-models crates/hyprlayer-proto)
NEW_SERVER=1
if git rev-parse -q --verify "refs/tags/$SERVER_TAG" >/dev/null; then
  NEW_SERVER=0
  if ! git diff --quiet "$SERVER_TAG" HEAD -- "${SERVER_PATHS[@]}"; then
    echo "✗ server code changed since $SERVER_TAG, but crates/hyprlayer-server is still $SERVER_VERSION."
    echo "  Bump its version, and raise MIN_SERVER_VERSION in src-tauri/src/server_version.rs"
    echo "  if this app needs the change. Changed:"
    git diff --stat "$SERVER_TAG" HEAD -- "${SERVER_PATHS[@]}" | tail -n 5
    exit 1
  fi
  echo "→ hyprlayer-server $SERVER_VERSION unchanged since $SERVER_TAG — no server release"
else
  echo "→ hyprlayer-server $SERVER_VERSION is new — this release publishes $SERVER_TAG"
fi

CUR="$(node -p 'require("./package.json").version')"
case "$BUMP" in
  patch) NEW=$(awk -F. -v OFS=. '{$3++; print}' <<<"$CUR") ;;
  minor) NEW=$(awk -F. -v OFS=. '{$2++; $3=0; print}' <<<"$CUR") ;;
  major) NEW=$(awk -F. -v OFS=. '{$1++; $2=0; $3=0; print}' <<<"$CUR") ;;
  *)     NEW="$BUMP" ;;
esac
echo "→ Bumping $CUR → $NEW"

CL_STATUS="$(node -e '
  const fs = require("fs");
  const f = "changelog.json";
  const v = process.argv[1];
  let j;
  try { j = JSON.parse(fs.readFileSync(f, "utf8")); }
  catch (e) {
    if (e.code === "ENOENT") j = { versions: [] };
    else { console.error("✗ Failed to parse changelog.json:"); throw e; }
  }
  if (!Array.isArray(j.versions)) j.versions = [];
  let top = j.versions[0];
  if (!top || top.version !== v) {
    top = { version: v, date: "", summary: "", notes: [""] };
    j.versions.unshift(top);
  }
  top.date = new Date().toISOString().slice(0, 10);
  fs.writeFileSync(f, JSON.stringify(j, null, 2) + "\n");
  const haveSummary = typeof top.summary === "string" && top.summary.trim().length > 0;
  const haveNotes = Array.isArray(top.notes) && top.notes.some(n => typeof n === "string" && n.trim().length > 0);
  if (!haveSummary || !haveNotes) { process.stdout.write("INCOMPLETE"); process.exit(0); }
  if (top.summary.trim().split(/\s+/).length > 15)
    process.stderr.write("  ⚠ summary >15 words — it renders in a narrow sidebar card.\n");
  process.stdout.write("OK");
' "$NEW")"
if [[ "$CL_STATUS" != "OK" ]]; then
  echo ""
  echo "✗ changelog.json needs the $NEW entry filled in."
  echo "  A stub for $NEW is now at the top of changelog.json — write its short"
  echo "  \"summary\" (≤15 words) and at least one \"notes\" bullet. Both are"
  echo "  user-facing — they head the GitHub Release page and the in-app card."
  echo "  Then re-run:"
  echo "      ./scripts/release.sh $BUMP"
  exit 1
fi
echo "→ changelog.json entry for $NEW: ok"

set_json_version() {
  node -e "const fs=require('fs');const f=process.argv[1];const j=JSON.parse(fs.readFileSync(f));j.version=process.argv[2];fs.writeFileSync(f,JSON.stringify(j,null,2)+'\n')" "$1" "$NEW"
}
set_json_version package.json
set_json_version src-tauri/tauri.conf.json

sed -i.bak -E "s/^version = \".*\"/version = \"$NEW\"/" Cargo.toml
rm Cargo.toml.bak

cargo update -p hyprlayer-app -p hyprlayer-core >/dev/null 2>&1 || true

git add package.json Cargo.toml Cargo.lock src-tauri/tauri.conf.json changelog.json
git commit -m "release: v$NEW"
git tag "v$NEW"
[[ "$NEW_SERVER" == "1" ]] && git tag "$SERVER_TAG"

echo ""
echo "✓ Tagged v$NEW."
[[ "$NEW_SERVER" == "1" ]] && echo "✓ Tagged $SERVER_TAG."
echo "  Publish (all three):   ./scripts/release-local.sh"
echo "  Publish (skip Linux):  ./scripts/release-local.sh --no-linux"
echo "  Publish (mac only):    ./scripts/release-local.sh --mac-only"
