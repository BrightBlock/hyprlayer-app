#!/usr/bin/env bash
set -euo pipefail

# dev-install-*.sh --remote: installs the same commit on a Linux host and makes
# its server that host's version-keyed slot and this app's cached copy, so both
# apps share one daemon. The host alias must pin this app's profile (forced
# command), or stopping its server would end the release profile's sessions.

TRIPLE="x86_64-unknown-linux-gnu"
HOST="" REF="" APP_ID="" VERSION="" CACHE_ROOT=""
PASS=()

step() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
info() { printf '    %s\n' "$*"; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --host)       HOST="${2:?}"; shift 2 ;;
    --ref)        REF="${2:?}"; shift 2 ;;
    --app-id)     APP_ID="${2:?}"; shift 2 ;;
    --version)    VERSION="${2:?}"; shift 2 ;;
    --cache-root) CACHE_ROOT="${2:?}"; shift 2 ;;
    --force|--no-version-suffix) PASS+=("$1"); shift ;;
    *) die "unknown argument: $1" ;;
  esac
done
[[ -n "$HOST" && -n "$REF" && -n "$APP_ID" && -n "$VERSION" && -n "$CACHE_ROOT" ]] \
  || die "usage: dev-install-remote.sh --host H --ref SHA --app-id ID --version V --cache-root DIR"
SCRIPT="$(cd "$(dirname "$0")" && pwd)/dev-install-linux.sh"

step "Checking $HOST pins the $APP_ID profile"
PROBE="$(ssh -o BatchMode=yes -o ConnectTimeout=10 "$HOST" \
  'printf "%s\n%s\n" "${HYPRLAYER_APP_DATA_DIR:-}" "$(uname -sm)"')" \
  || die "$HOST unreachable over SSH"
PROFILE="$(sed -n 1p <<<"$PROBE")"
UNAME="$(sed -n 2p <<<"$PROBE")"
[[ "${PROFILE##*/}" == "$APP_ID" ]] \
  || die "$HOST runs commands in the '${PROFILE:-default}' profile, not $APP_ID — give it a key whose forced command exports HYPRLAYER_APP_{CONFIG,DATA,LOG}_DIR for $APP_ID"
[[ "$UNAME" == "Linux x86_64" ]] || die "$HOST is '$UNAME'; only $TRIPLE hosts are supported"

step "Running dev-install-linux.sh on $HOST at ${REF:0:7}"
ssh "$HOST" 'mkdir -p ~/.cache && cat > ~/.cache/dev-install-linux.sh' < "$SCRIPT"
# shellcheck disable=SC2029  # the arguments are meant to expand on this side.
ssh "$HOST" "bash -lc 'bash ~/.cache/dev-install-linux.sh --ref $REF ${PASS[*]:-} </dev/null'" \
  || die "dev-install-linux.sh failed on $HOST (is ${REF:0:7} pushed?)"

step "Installing its server as $HOST's $VERSION slot"
# shellcheck disable=SC2029
SLOT="$(ssh "$HOST" "set -e; src=\"\$HOME/.local/bin/hyprlayer-server\"; \
  v=\"\$(\"\$src\" --version)\"; [ \"\$v\" = '$VERSION' ] || { echo \"the host built \$v, not $VERSION\" >&2; exit 1; }; \
  r=\"\$HOME/.hyprlayer/server\"; d=\"\$r/$VERSION\"; \
  new=\"\$(sha256sum \"\$src\" | awk '{print \$1}')\"; \
  old=\"\$(sha256sum \"\$d/hyprlayer-server\" 2>/dev/null | awk '{print \$1}')\"; \
  if [ \"\$new\" != \"\$old\" ]; then rm -rf \"\$d.tmp\"; mkdir -p \"\$d.tmp\"; \
    cp \"\$src\" \"\$d.tmp/hyprlayer-server\"; chmod 755 \"\$d.tmp/hyprlayer-server\"; \
    rm -rf \"\$d\"; mv \"\$d.tmp\" \"\$d\"; echo changed; fi; echo \"sha \$new\"")" \
  || die "could not install the slot on $HOST"
SHA="$(sed -n 's/^sha //p' <<<"$SLOT")"

# readiness() calls the slot Stale unless this cache holds the same bytes.
step "Caching the same bytes for this app"
CACHED="$CACHE_ROOT/$VERSION/hyprlayer-server-$TRIPLE"
mkdir -p "$(dirname "$CACHED")"
# shellcheck disable=SC2029
ssh "$HOST" "cat \"\$HOME/.hyprlayer/server/$VERSION/hyprlayer-server\"" > "$CACHED.tmp"
[[ "$({ sha256sum "$CACHED.tmp" 2>/dev/null || shasum -a 256 "$CACHED.tmp"; } | awk '{print $1}')" == "$SHA" ]] \
  || { rm -f "$CACHED.tmp"; die "the copied-down server does not match the host's sha"; }
chmod 755 "$CACHED.tmp"
mv -f "$CACHED.tmp" "$CACHED"
info "$CACHED"

if grep -qx changed <<<"$SLOT"; then
  step "Stopping $HOST's $APP_ID server so the next connect starts $VERSION"
  info "this ends that profile's sessions and terminals, from every app attached to it"
  # shellcheck disable=SC2029
  ssh "$HOST" "b=\"\$HOME/.hyprlayer/server/$VERSION/hyprlayer-server\"; \
    \"\$b\" status >/dev/null 2>&1 || exit 0; \"\$b\" stop >&2; i=0; \
    while \"\$b\" status >/dev/null 2>&1; do \
    [ \$i -ge 50 ] && { echo 'the server did not stop within 10s' >&2; exit 1; }; \
    sleep 0.2; i=\$((i+1)); done" \
    || die "the server on $HOST did not stop"
else
  info "$HOST already had these bytes; its server was left running"
fi
