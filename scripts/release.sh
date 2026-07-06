#!/usr/bin/env bash
# Cut a release tag locally. Then run ./scripts/release-local.sh to build +
# ed25519-sign the macOS arm64 and Windows x64 bundles, upload a GitHub Release,
# and write the updater manifest + changelog to brightblock.ai. Running apps see
# the new version via the in-app Update pill within a few minutes.
# (There is no CI release — GitHub Actions was removed; releases are local-only.)
#
# Before bumping versions this script gates on changelog.json: the entry for
# the new version must exist with a non-empty summary (it's what the in-app
# Update card renders and what release-local.sh copies to brightblock.ai). A stub is
# scaffolded if missing — fill it in and re-run.
#
# Usage:
#   ./scripts/release.sh patch     0.1.0 → 0.1.1
#   ./scripts/release.sh minor     0.1.0 → 0.2.0
#   ./scripts/release.sh major     0.1.0 → 1.0.0
#   ./scripts/release.sh 0.4.2     set an explicit version
set -euo pipefail

cd "$(dirname "$0")/.."
BUMP="${1:-patch}"

# ─── preflight ──────────────────────────────────────────────────────
# changelog.json is authored right before the release, so it's expected to be
# uncommitted here — exempt it from the dirty check. release.sh commits it
# together with the version bump below.
if [[ -n "$(git status --porcelain | grep -v ' changelog\.json$' || true)" ]]; then
  echo "✗ working tree dirty (other than changelog.json) — commit or stash first"
  exit 1
fi
BRANCH="$(git rev-parse --abbrev-ref HEAD)"
if [[ "$BRANCH" != "main" ]]; then
  echo "✗ releases are cut from main only (currently on $BRANCH)"
  exit 1
fi

# ─── verification gate ──────────────────────────────────────────────
# With no CI (releases are local-only by design), this is the enforcement
# point ADR-0001 names: cargo test regenerates src/lib/bindings.ts, so a
# drifted contract fails the dirty-tree check below; lint carries the
# ADR-0003 import-boundary rules. Skip only with RELEASE_SKIP_CHECKS=1.
if [[ "${RELEASE_SKIP_CHECKS:-}" != "1" ]]; then
  echo "→ Verification gate (cargo test, typecheck, lint, vitest)…"
  cargo test --workspace --quiet
  pnpm typecheck && pnpm lint && pnpm test
  if [[ -n "$(git status --porcelain -- src/lib/bindings.ts)" ]]; then
    echo "✗ src/lib/bindings.ts drifted — cargo test regenerated it. Commit the diff first."
    exit 1
  fi
else
  echo "⚠ RELEASE_SKIP_CHECKS=1 — skipping the verification gate"
fi

# ─── compute new version ────────────────────────────────────────────
CUR="$(node -p 'require("./package.json").version')"
case "$BUMP" in
  patch) NEW=$(awk -F. -v OFS=. '{$3++; print}' <<<"$CUR") ;;
  minor) NEW=$(awk -F. -v OFS=. '{$2++; $3=0; print}' <<<"$CUR") ;;
  major) NEW=$(awk -F. -v OFS=. '{$1++; $2=0; $3=0; print}' <<<"$CUR") ;;
  *)     NEW="$BUMP" ;;
esac
echo "→ Bumping $CUR → $NEW"

# ─── changelog gate ─────────────────────────────────────────────────
# Scaffold a stub for $NEW if missing, stamp today's date, and refuse to
# proceed until its summary is written.
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
  echo "  \"summary\" (≤15 words) and at least one \"notes\" bullet, then re-run:"
  echo "      ./scripts/release.sh $BUMP"
  exit 1
fi
echo "→ changelog.json entry for $NEW: ok"

# ─── bump the three version sources in lockstep ─────────────────────
# Both JSON files are rewritten the same way — parse, set .version, write with
# a trailing newline — rather than shelling out to `npm version` (which also
# runs lifecycle hooks and normalizes the file).
set_json_version() {
  node -e "const fs=require('fs');const f=process.argv[1];const j=JSON.parse(fs.readFileSync(f));j.version=process.argv[2];fs.writeFileSync(f,JSON.stringify(j,null,2)+'\n')" "$1" "$NEW"
}
set_json_version package.json
set_json_version src-tauri/tauri.conf.json

# Root Cargo.toml → [workspace.package] version (src-tauri inherits it via
# version.workspace = true).
sed -i.bak -E "s/^version = \".*\"/version = \"$NEW\"/" Cargo.toml
rm Cargo.toml.bak

# Keep Cargo.lock's package entries in sync with the bumped versions.
cargo update -p hyprlayer-app -p hyprlayer-core >/dev/null 2>&1 || true

# ─── commit + tag ──────────────────────────────────────────────────
git add package.json Cargo.toml Cargo.lock src-tauri/tauri.conf.json changelog.json
git commit -m "release: v$NEW"
git tag "v$NEW"

echo ""
echo "✓ Tagged v$NEW."
echo "  Publish (mac + win):  ./scripts/release-local.sh"
echo "  Publish (mac only):   ./scripts/release-local.sh --mac-only"
