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
# No checksum is written or verified: these bytes came off your own builder and
# are cross-checked against the sha it reported, the same provenance the app
# itself has. Verifying them against a checksum generated from the same bytes
# would prove nothing.

LINUX_TRIPLE="x86_64-unknown-linux-gnu"
MAC_TRIPLE="aarch64-apple-darwin"
TRIPLE="$LINUX_TRIPLE"
# Empty means every identifier below. Staging to one is the trap this tool
# exists to avoid: the default used to be the *release* identifier while the
# build a developer actually runs is `.dev`, so the binary landed in a cache
# the app never reads, and the app fell through to a download that does not
# exist yet — which reads as provisioning being broken rather than as staging
# to the wrong place. The bytes are identical for all three, so seed them all.
IDENTIFIER=""
ALL_IDENTIFIERS=(
  "ai.brightblock.hyprlayer"      # release install
  "ai.brightblock.hyprlayer.dev"  # tauri.dev.conf.json
  "ai.brightblock.hyprlayer.main" # dev-install-*.sh
)
BUILDER="${HYPRLAYER_LINUX_SSH_HOST:-nexus}"
REMOTE_DIR="hyprlayer-server-stage"
FORCE=0
FROM_HEAD=0
VERSION_OVERRIDE=""
# dev-install-*.sh all build out of this clone, stamping the server's version
# `<version>+dev.<sha>` first — so it records what the installed app asks for.
DEV_CLONE="${HYPRLAYER_DEV_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/hyprlayer-dev}/src"
DEV_IDENTIFIER="ai.brightblock.hyprlayer.main"

say() { printf '\033[1;36m→ %s\033[0m\n' "$*"; }
ok()  { printf '\033[1;32m✓ %s\033[0m\n' "$*"; }
die() { printf '\033[1;31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

usage() {
  printf '%s\n' \
    'stage-server-binary.sh - build hyprlayer-server and seed the client cache' \
    '' \
    "  --triple <triple>     $LINUX_TRIPLE (default) or $MAC_TRIPLE" \
    '  --identifier <id>     stage for one app identifier instead of all three' \
    '                        ai.brightblock.hyprlayer (the release),' \
    '                        ai.brightblock.hyprlayer.dev (tauri.dev.conf.json),' \
    '                        ai.brightblock.hyprlayer.main (dev-install-*.sh)' \
    '                        Default: all three — the bytes are identical, and' \
    '                        staging only one means the build you actually run' \
    '                        may read an empty cache.' \
    '  --version <v>         key the cache at this version for every identifier' \
    '                        instead of the one each app reports. Default: the' \
    "                        tree's hyprlayer-server version, except for" \
    '                        ai.brightblock.hyprlayer.main, which is read from' \
    '                        the dev-install build clone.' \
    '  --force               rebuild even when the cache already holds it' \
    '  --from-head           build the committed HEAD instead of the working tree' \
    '' \
    "  HYPRLAYER_LINUX_SSH_HOST  builder for $LINUX_TRIPLE (default: nexus)" \
    '  HYPRLAYER_APP_DATA_DIR    override the resolved app data dir' \
    '  HYPRLAYER_DEV_CACHE       dev-install build clone (default: ~/.cache/hyprlayer-dev)'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --triple)     TRIPLE="${2:?--triple needs a value}"; shift 2 ;;
    --identifier) IDENTIFIER="${2:?--identifier needs a value}"; shift 2 ;;
    --version)    VERSION_OVERRIDE="${2:?--version needs a value}"; shift 2 ;;
    --force)      FORCE=1; shift ;;
    --from-head)  FROM_HEAD=1; shift ;;
    -h|--help)    usage; exit 0 ;;
    *)            die "unknown argument: $1 (see --help)" ;;
  esac
done

for tool in cargo rustc shasum; do
  command -v "$tool" >/dev/null || die "missing required tool: $tool"
done

manifest_version() { sed -n 's/^version = "\(.*\)"$/\1/p' "$1" | head -n1; }

# The server is versioned apart from the app (ADR-0009), and its version is what
# the client keys the cache on.
SERVER_MANIFEST="crates/hyprlayer-server/Cargo.toml"

# What this tree builds, so what the builder's binary must report.
VERSION="$(manifest_version "$SERVER_MANIFEST")"
[[ -n "$VERSION" ]] || die "could not read the server version from $SERVER_MANIFEST"

# What the app asks for, which is a different question: a dev-install build
# carries `+dev.<sha>`, so it reads a cache directory this tree's version never
# names — the version half of the trap above, and the half that reads as a 404.
cache_version_for() {
  local built
  if [[ -n "$VERSION_OVERRIDE" ]]; then
    printf '%s' "$VERSION_OVERRIDE"
  elif [[ "$1" == "$DEV_IDENTIFIER" && -f "$DEV_CLONE/$SERVER_MANIFEST" ]] \
    && built="$(manifest_version "$DEV_CLONE/$SERVER_MANIFEST")" && [[ -n "$built" ]]; then
    # Only the build metadata may differ: a clone left on another release builds
    # a server whose protocol this tree's bytes need not speak.
    if [[ "${built%%+*}" == "${VERSION%%+*}" ]]; then
      printf '%s' "$built"
    else
      printf '! %s is installed from %s, not this tree (%s) — staging it as %s; rerun dev-install, or pass --version\n' \
        "$1" "$built" "$VERSION" "$VERSION" >&2
      printf '%s' "$VERSION"
    fi
  else
    printf '%s' "$VERSION"
  fi
}

data_dir_for() {
  case "$(uname -s)" in
    Darwin) printf '%s' "$HOME/Library/Application Support/$1" ;;
    Linux)  printf '%s' "${XDG_DATA_HOME:-$HOME/.local/share}/$1" ;;
    *)      die "unsupported client platform: $(uname -s) (set HYPRLAYER_APP_DATA_DIR)" ;;
  esac
}

if [[ -n "$IDENTIFIER" ]]; then
  TARGET_IDENTIFIERS=("$IDENTIFIER")
else
  TARGET_IDENTIFIERS=("${ALL_IDENTIFIERS[@]}")
fi

CACHES=()
for id in "${TARGET_IDENTIFIERS[@]}"; do
  # An explicit override names one directory, so it cannot stand in for several.
  if [[ -n "${HYPRLAYER_APP_DATA_DIR:-}" ]]; then
    [[ ${#TARGET_IDENTIFIERS[@]} -eq 1 ]] \
      || die "HYPRLAYER_APP_DATA_DIR names a single directory — pass --identifier too"
    DATA_DIR="$HYPRLAYER_APP_DATA_DIR"
  else
    DATA_DIR="$(data_dir_for "$id")"
  fi
  CACHES+=("$DATA_DIR/server-cache/$(cache_version_for "$id")/hyprlayer-server-$TRIPLE")
done

MISSING=()
for c in "${CACHES[@]}"; do [[ -f "$c" ]] || MISSING+=("$c"); done
if [[ "$FORCE" != "1" && ${#MISSING[@]} -eq 0 ]]; then
  ok "Already staged for ${#CACHES[@]} identifier(s):"
  printf '    %s\n' "${CACHES[@]}"
  exit 0
fi

HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
[[ -n "$HOST_TRIPLE" ]] || die "could not read the host triple from \`rustc -vV\`"

TMPDIRS=()
cleanup() { for d in "${TMPDIRS[@]:-}"; do [[ -n "${d:-}" ]] && rm -rf "$d"; done; }
trap cleanup EXIT
TMP="$(mktemp -d)"; TMPDIRS+=("$TMP")
BUILT="$TMP/hyprlayer-server"

if [[ "$TRIPLE" == "$HOST_TRIPLE" ]]; then
  say "Building hyprlayer-server $VERSION for $TRIPLE locally…"
  cargo build --release -p hyprlayer-server
  LOCAL_BIN="${CARGO_TARGET_DIR:-target}/release/hyprlayer-server"
  [[ -x "$LOCAL_BIN" ]] || die "expected binary not found: $LOCAL_BIN"
  cp "$LOCAL_BIN" "$BUILT"
elif [[ "$TRIPLE" == "$LINUX_TRIPLE" ]]; then
  REQUIRED=(ssh scp)
  if [[ "$FROM_HEAD" == "1" ]]; then REQUIRED+=(git); else REQUIRED+=(rsync); fi
  for tool in "${REQUIRED[@]}"; do
    command -v "$tool" >/dev/null || die "missing required tool: $tool"
  done
  say "Checking the Linux builder ($BUILDER) is reachable…"
  ssh -o BatchMode=yes -o ConnectTimeout=10 "$BUILDER" "echo ok" >/dev/null 2>&1 \
    || die "builder '$BUILDER' unreachable over SSH — wake it / check Tailscale, or set HYPRLAYER_LINUX_SSH_HOST"

  if [[ "$FROM_HEAD" == "1" ]]; then
    say "Shipping HEAD to $BUILDER…"
    git archive --format=tar.gz -o "$TMP/src.tgz" HEAD
    scp -q "$TMP/src.tgz" "$BUILDER:hyprlayer-server-src.tgz"
    # shellcheck disable=SC2029  # $REMOTE_DIR is meant to expand on this side.
    ssh "$BUILDER" "rm -rf '$REMOTE_DIR' && mkdir -p '$REMOTE_DIR' \
      && tar -xzf hyprlayer-server-src.tgz -C '$REMOTE_DIR'" \
      || die "could not unpack the source tarball on $BUILDER"
  else
    say "Syncing the working tree to $BUILDER…"
    # `.git` is a file in a worktree, so no trailing slash on that one.
    rsync -a --delete \
      --exclude '.git' --exclude 'target/' --exclude 'node_modules/' \
      --exclude 'dist/' --exclude 'src-tauri/target/' --exclude 'src-tauri/binaries/' \
      ./ "$BUILDER:$REMOTE_DIR/" \
      || die "could not sync the working tree to $BUILDER"
  fi

  HELPER="$TMP/stage-server-remote.sh"
  cat > "$HELPER" <<REMOTE
#!/usr/bin/env bash
set -euo pipefail
# The release builder's target dir, so a staging build reuses its warm cache
# (and vice versa) instead of compiling the world twice.
export CARGO_TARGET_DIR="\${CARGO_TARGET_DIR:-\$HOME/.hyprlayer-linux-target}"
command -v cargo >/dev/null || { echo "missing cargo on the builder" >&2; exit 1; }
cd "\$HOME/$REMOTE_DIR"
cargo build --release -p hyprlayer-server
BIN="\$CARGO_TARGET_DIR/release/hyprlayer-server"
[ -x "\$BIN" ] || { echo "no hyprlayer-server at \$BIN" >&2; exit 1; }
echo "SERVER_BIN=\$BIN"
echo "SERVER_SHA=\$(sha256sum "\$BIN" | awk '{print \$1}')"
# Reported only when the build accepts --version; \`echo "X=\$(failing)"\` would
# swallow the failure under set -e and hand back an empty value.
if OUT="\$("\$BIN" --version 2>/dev/null)"; then echo "SERVER_VERSION=\$OUT"; fi
REMOTE

  say "Building hyprlayer-server $VERSION on $BUILDER…"
  scp -q "$HELPER" "$BUILDER:stage-server-remote.sh"
  LOG="$TMP/build.log"
  if ! ssh "$BUILDER" "bash -lc 'bash ~/stage-server-remote.sh'" >"$LOG" 2>&1; then
    cat "$LOG" >&2
    die "the hyprlayer-server build failed on $BUILDER (log above)"
  fi
  remote_line() { grep -E "^$1=" "$LOG" | tail -n1 | sed "s/^$1=//" | tr -d '\r'; }
  REMOTE_BIN="$(remote_line SERVER_BIN)"
  REMOTE_SHA="$(remote_line SERVER_SHA)"
  REMOTE_VERSION="$(remote_line SERVER_VERSION)"
  [[ -n "$REMOTE_BIN" && -n "$REMOTE_SHA" ]] \
    || { cat "$LOG" >&2; die "the builder printed no SERVER_BIN/SERVER_SHA line"; }
  scp -q "$BUILDER:$REMOTE_BIN" "$BUILT" \
    || die "failed to copy hyprlayer-server down from $BUILDER"
  [[ "$(shasum -a 256 "$BUILT" | awk '{print $1}')" == "$REMOTE_SHA" ]] \
    || die "the copied-down hyprlayer-server does not match the sha the builder reported"
  if [[ -z "$REMOTE_VERSION" ]]; then
    printf '! the build does not accept --version, so the cache key is unverified\n' >&2
  elif [[ "$REMOTE_VERSION" != "$VERSION" ]]; then
    die "the builder produced hyprlayer-server $REMOTE_VERSION but the cache is keyed $VERSION"
  fi
else
  die "no builder for $TRIPLE on a $HOST_TRIPLE client (Windows remotes are not provisioned yet)"
fi

for c in "${CACHES[@]}"; do
  mkdir -p "$(dirname "$c")"
  install -m 755 "$BUILT" "$c.tmp"
  mv -f "$c.tmp" "$c"
done
ok "Staged hyprlayer-server $VERSION ($TRIPLE) into ${#CACHES[@]} cache(s):"
printf '    %s\n' "${CACHES[@]}"
