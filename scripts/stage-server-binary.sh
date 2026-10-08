#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

# Builds hyprlayer-server for one target and drops it where the client looks
# before it downloads (plan Phase 7 §1). The staging destination *is* the
# download cache, so seeding it here means the resolver has one lookup rather
# than a dev branch and a real branch.
#
# It ships your *working tree*, not a commit: you stage a binary to exercise the
# server change you are working on, and a release is the thing that has to be
# reproducible from a commit. `--from-head` is the reproducible mode.
#
# The Linux server builds in the release's Linux builder container
# (scripts/builders/run.sh --server-only), sharing its warm volumes and its
# glibc 2.34 floor. No checksum is written: these bytes come from your own
# build, the same provenance the app itself has.

LINUX_TRIPLE="x86_64-unknown-linux-gnu"
MAC_TRIPLE="aarch64-apple-darwin"
TRIPLE="$LINUX_TRIPLE"
# Empty means every identifier below, less the release once its version is
# tagged (see the cache targets). Staging to one is the trap this tool
# exists to avoid: the default used to be the *release* identifier while the
# build a developer actually runs is `.dev`, so the binary landed in a cache
# the app never reads, and the app fell through to a download that does not
# exist yet — which reads as provisioning being broken rather than as staging
# to the wrong place. The bytes are identical for both, so seed both.
# ai.brightblock.hyprlayer.main (dev-install-*.sh) asks for its own commit's
# server, which this tree is not: `dev-install-*.sh --remote` stages that one.
IDENTIFIER=""
RELEASE_IDENTIFIER="ai.brightblock.hyprlayer"
ALL_IDENTIFIERS=(
  "$RELEASE_IDENTIFIER"           # release install
  "ai.brightblock.hyprlayer.dev"  # tauri.dev.conf.json
)
DATA_DIR_OVERRIDE=""
FORCE=0
FROM_HEAD=0
VERSION_OVERRIDE=""
DEV_IDENTIFIER="ai.brightblock.hyprlayer.main"

say() { printf '\033[1;36m→ %s\033[0m\n' "$*"; }
ok()  { printf '\033[1;32m✓ %s\033[0m\n' "$*"; }
die() { printf '\033[1;31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

usage() {
  printf '%s\n' \
    'stage-server-binary.sh - build hyprlayer-server and seed the client cache' \
    '' \
    "  --triple <triple>     $LINUX_TRIPLE (default) or $MAC_TRIPLE" \
    '  --identifier <id>     stage for one app identifier instead of both' \
    '                        ai.brightblock.hyprlayer (the release),' \
    '                        ai.brightblock.hyprlayer.dev (tauri.dev.conf.json)' \
    '                        Default: both, but the release only while' \
    '                        server-v<version> is untagged. For dev-install' \
    '                        builds (ai.brightblock.hyprlayer.main) use' \
    '                        dev-install-*.sh --remote <host> instead.' \
    '  --version <v>         key the cache at this version instead of the' \
    "                        tree's hyprlayer-server version" \
    '  --force               rebuild even when the cache already holds it' \
    "  --data-dir <dir>      stage into this app data dir (absolute) instead of an identifier's" \
    '  --from-head           build the committed HEAD instead of the working tree;' \
    '                        container builds only (the default triple on a Mac)' \
    '' \
    'The Linux build runs in Docker. HYPRLAYER_APP_DATA_DIR is not read; use --data-dir.'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --triple)     TRIPLE="${2:?--triple needs a value}"; shift 2 ;;
    --identifier) IDENTIFIER="${2:?--identifier needs a value}"; shift 2 ;;
    --data-dir)   DATA_DIR_OVERRIDE="${2:?--data-dir needs a value}"; shift 2 ;;
    --version)    VERSION_OVERRIDE="${2:?--version needs a value}"; shift 2 ;;
    --force)      FORCE=1; shift ;;
    --from-head)  FROM_HEAD=1; shift ;;
    -h|--help)    usage; exit 0 ;;
    *)            die "unknown argument: $1 (see --help)" ;;
  esac
done

data_dir_for() {
  case "$(uname -s)" in
    Darwin) printf '%s' "$HOME/Library/Application Support/$1" ;;
    Linux)  printf '%s' "${XDG_DATA_HOME:-$HOME/.local/share}/$1" ;;
    *)      die "unsupported client platform: $(uname -s) (pass --data-dir)" ;;
  esac
}

[[ -z "$DATA_DIR_OVERRIDE" || -z "$IDENTIFIER" ]] || die "pass --data-dir or --identifier, not both"
[[ -z "$DATA_DIR_OVERRIDE" || "$DATA_DIR_OVERRIDE" == /* ]] || die "--data-dir needs an absolute path"
DATA_DIR_NAME=""
[[ -z "$DATA_DIR_OVERRIDE" ]] || DATA_DIR_NAME="$(basename "$DATA_DIR_OVERRIDE")"
[[ "$IDENTIFIER" != "$DEV_IDENTIFIER" && "$DATA_DIR_NAME" != "$DEV_IDENTIFIER" ]] \
  || die "$DEV_IDENTIFIER runs its own commit's server, not this tree's — use dev-install-*.sh --remote <host>"
# A shell may export the release dir for hand-run servers; any other value was meant as a target.
[[ -z "${HYPRLAYER_APP_DATA_DIR:-}" || -n "$DATA_DIR_OVERRIDE$IDENTIFIER" \
   || "${HYPRLAYER_APP_DATA_DIR%/}" == "$(data_dir_for "$RELEASE_IDENTIFIER")" ]] \
  || die "HYPRLAYER_APP_DATA_DIR is not read here — pass --data-dir \"$HYPRLAYER_APP_DATA_DIR\""

command -v rustc >/dev/null || die "missing required tool: rustc"
HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
[[ -n "$HOST_TRIPLE" ]] || die "could not read the host triple from \`rustc -vV\`"
if [[ "$TRIPLE" == "$HOST_TRIPLE" ]]; then
  MODE=local
  REQUIRED=(cargo)
elif [[ "$TRIPLE" == "$LINUX_TRIPLE" ]]; then
  MODE=container
  REQUIRED=(git docker node)
else
  die "no builder for $TRIPLE on a $HOST_TRIPLE client (Windows remotes are not provisioned yet)"
fi
for tool in "${REQUIRED[@]}"; do
  command -v "$tool" >/dev/null || die "missing required tool: $tool"
done
[[ "$FROM_HEAD" != "1" || "$MODE" == container ]] \
  || die "--from-head needs the container build; $TRIPLE on this $HOST_TRIPLE machine builds the working tree with local cargo"

manifest_version() { sed -n 's/^version = "\(.*\)"$/\1/p' | head -n1; }

# The server is versioned apart from the app (ADR-0009), and its version is what
# the client keys the cache on; --from-head builds HEAD, so it reads HEAD's.
SERVER_MANIFEST="crates/hyprlayer-server/Cargo.toml"
if [[ "$FROM_HEAD" == "1" ]]; then
  VERSION="$(git show "HEAD:$SERVER_MANIFEST" | manifest_version)"
else
  VERSION="$(manifest_version < "$SERVER_MANIFEST")"
fi
[[ -n "$VERSION" ]] || die "could not read the server version from $SERVER_MANIFEST"
CACHE_VERSION="${VERSION_OVERRIDE:-$VERSION}"

if [[ -n "$DATA_DIR_OVERRIDE" ]]; then
  DATA_DIRS=("$DATA_DIR_OVERRIDE")
else
  if [[ -n "$IDENTIFIER" ]]; then
    TARGET_IDENTIFIERS=("$IDENTIFIER")
  elif git rev-parse -q --verify "refs/tags/server-v$CACHE_VERSION" >/dev/null 2>&1; then
    # The release app prefers its cache to the download.
    say "Skipping the release cache: server-v$CACHE_VERSION is released, so a staged build would shadow it (pass --identifier $RELEASE_IDENTIFIER to stage there)"
    TARGET_IDENTIFIERS=()
    for id in "${ALL_IDENTIFIERS[@]}"; do
      [[ "$id" == "$RELEASE_IDENTIFIER" ]] || TARGET_IDENTIFIERS+=("$id")
    done
  else
    TARGET_IDENTIFIERS=("${ALL_IDENTIFIERS[@]}")
  fi
  DATA_DIRS=()
  for id in "${TARGET_IDENTIFIERS[@]}"; do
    dir="$(data_dir_for "$id")"
    DATA_DIRS+=("$dir")
  done
fi
CACHES=()
for dir in "${DATA_DIRS[@]}"; do
  CACHES+=("$dir/server-cache/$CACHE_VERSION/hyprlayer-server-$TRIPLE")
done

MISSING=()
for c in "${CACHES[@]}"; do [[ -f "$c" ]] || MISSING+=("$c"); done
if [[ "$FORCE" != "1" && ${#MISSING[@]} -eq 0 ]]; then
  ok "Already staged in ${#CACHES[@]} cache(s):"
  printf '    %s\n' "${CACHES[@]}"
  exit 0
fi

TMPDIRS=()
cleanup() { for d in "${TMPDIRS[@]:-}"; do [[ -n "${d:-}" ]] && rm -rf "$d"; done; }
trap cleanup EXIT
TMP="$(mktemp -d)"; TMPDIRS+=("$TMP")
BUILT="$TMP/hyprlayer-server"

if [[ "$MODE" == local ]]; then
  say "Building hyprlayer-server $VERSION for $TRIPLE locally…"
  cargo build --release -p hyprlayer-server
  LOCAL_BIN="${CARGO_TARGET_DIR:-target}/release/hyprlayer-server"
  [[ -x "$LOCAL_BIN" ]] || die "expected binary not found: $LOCAL_BIN"
  cp "$LOCAL_BIN" "$BUILT"
else
  docker info >/dev/null 2>&1 \
    || die "Docker is not running — the $TRIPLE server builds in the Linux builder container"
  SRC="$TMP/hyprlayer-src.tgz"
  if [[ "$FROM_HEAD" == "1" ]]; then
    say "Archiving HEAD…"
    git archive --format=tar.gz -o "$SRC" HEAD
  else
    say "Archiving the working tree…"
    # What `git add -A` would commit, through a scratch index so yours is untouched.
    INDEX="$TMP/index"
    cp "$(git rev-parse --git-path index)" "$INDEX"
    GIT_INDEX_FILE="$INDEX" git add -A
    git archive --format=tar.gz -o "$SRC" "$(GIT_INDEX_FILE="$INDEX" git write-tree)"
  fi
  say "Building hyprlayer-server $VERSION for $TRIPLE in the Linux builder container…"
  LOG="$TMP/build.log"
  scripts/builders/run.sh linux "$SRC" "$TMP/out" --server-only 2>&1 | tee "$LOG" \
    || die "the hyprlayer-server build failed in the Linux builder container (output above)"
  # `|| true`: no match must reach the check below, not end the script under pipefail.
  BUILT_VERSION="$({ grep -E '^SERVER_VERSION=' "$LOG" || true; } | tail -n1 | sed 's/^SERVER_VERSION=//' | tr -d '\r')"
  [[ "$BUILT_VERSION" == "$VERSION" ]] \
    || die "the container built hyprlayer-server ${BUILT_VERSION:-<unknown>} but the cache is keyed $VERSION"
  cp "$TMP/out/hyprlayer-server-$TRIPLE" "$BUILT"
fi

for c in "${CACHES[@]}"; do
  mkdir -p "$(dirname "$c")"
  install -m 755 "$BUILT" "$c.tmp"
  mv -f "$c.tmp" "$c"
done
SHA="$({ sha256sum "$BUILT" 2>/dev/null || shasum -a 256 "$BUILT"; } | awk '{print $1}')"
ok "Staged hyprlayer-server $VERSION ($TRIPLE, sha256 $SHA) into ${#CACHES[@]} cache(s):"
printf '    %s\n' "${CACHES[@]}"
