#!/usr/bin/env bash
# Installs the Linux packages on clean distro containers, and on Arch launches
# the installed app under Xvfb:
#   scripts/builders/smoke/linux.sh <packages-dir> [out-dir]
set -euo pipefail

cd "$(dirname "$0")/../../.."

PKGS="${1:?usage: linux.sh <packages-dir> [out-dir]}"
OUT="${2:-target/smoke/linux}"
VERSION="$(node -p 'require("./package.json").version')"
mkdir -p "$OUT"
PKGS="$(cd "$PKGS" && pwd)"
OUT="$(cd "$OUT" && pwd)"

# name, base image, install path
TARGETS=(
  "ubuntu-24.04 ubuntu:24.04 deb"
  "debian-13 debian:trixie-slim deb"
  "fedora fedora:latest rpm"
  "arch archlinux:base-devel aur"
)

PIDS=()
for t in "${TARGETS[@]}"; do
  read -r name base mode <<<"$t"
  (
    # A derived tag, because `docker run --platform` would re-point the shared base tag.
    printf 'FROM %s\n' "$base" | docker build --quiet --platform linux/amd64 -t "hyprlayer-smoke/$name" - >/dev/null
    docker run --rm --platform linux/amd64 --env "VERSION=$VERSION" \
      --volume "$PKGS:/pkg:ro" --volume "$OUT:/out" \
      --volume "$PWD/scripts/builders/smoke/linux-host.sh:/smoke.sh:ro" \
      "hyprlayer-smoke/$name" sh /smoke.sh "$mode"
  ) >"$OUT/$name.log" 2>&1 &
  PIDS+=("$!:$name")
done

failed=0
for p in "${PIDS[@]}"; do
  name="${p#*:}"
  if wait "${p%%:*}"; then
    printf '\033[1;32m✓ %s\033[0m\n' "$name"
  else
    printf '\033[1;31m✗ %s — %s\033[0m\n' "$name" "$OUT/$name.log"
    failed=1
  fi
  grep -E '^(PASS|FAIL)' "$OUT/$name.log" | sed 's/^/    /' || true
done
exit "$failed"
