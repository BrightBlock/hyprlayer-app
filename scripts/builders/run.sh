#!/usr/bin/env bash
# Builds one platform's release artifacts in its container on this Mac:
#   scripts/builders/run.sh <windows|linux> <source.tgz> <out-dir>
set -euo pipefail

cd "$(dirname "$0")/../.."

USAGE="usage: run.sh <windows|linux> <source.tgz> <out-dir>"
PLATFORM="${1:?$USAGE}"
SRC_TGZ="${2:?$USAGE}"
OUT="${3:?$USAGE}"

[[ "$PLATFORM" == "windows" || "$PLATFORM" == "linux" ]] || { echo "$USAGE" >&2; exit 1; }
[[ -f "$SRC_TGZ" ]] || { echo "no source tarball at $SRC_TGZ" >&2; exit 1; }
docker info >/dev/null 2>&1 || { echo "Docker is not running" >&2; exit 1; }

# The Mac's toolchain, so all three legs compile with the same rustc and pnpm.
RUST_VERSION="$(rustc -V | awk '{print $2}')"
PNPM_VERSION="$(node -p 'require("./package.json").packageManager.split("@")[1]')"
IMAGE="hyprlayer-builder/$PLATFORM"

echo "==> Preparing $IMAGE (rust $RUST_VERSION, pnpm $PNPM_VERSION)"
docker build --quiet --platform linux/arm64 \
  --build-arg "RUST_VERSION=$RUST_VERSION" --build-arg "PNPM_VERSION=$PNPM_VERSION" \
  --tag "$IMAGE" "scripts/builders/$PLATFORM" >/dev/null

IN="$(mktemp -d)"
trap 'rm -rf "$IN"' EXIT
cp "$SRC_TGZ" "$IN/hyprlayer-src.tgz"
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"

# Both legs cross-compile on arm64, so neither needs Rosetta. Named volumes,
# not bind mounts: a cargo target dir on a macOS file share is slow, and these
# stay warm across releases. The cache volume keeps tauri-bundler from
# re-downloading its NSIS plugin on every run.
docker run --rm --platform linux/arm64 \
  --volume "$IN:/in:ro" \
  --volume "$OUT:/out" \
  --volume "hyprlayer-builder-$PLATFORM-target:/target" \
  --volume "hyprlayer-builder-$PLATFORM-cargo:/usr/local/cargo/registry" \
  --volume "hyprlayer-builder-$PLATFORM-pnpm:/pnpm-store" \
  --volume "hyprlayer-builder-$PLATFORM-cache:/root/.cache" \
  "$IMAGE"

if [[ "$PLATFORM" == "linux" ]]; then
  echo "==> Rendering .SRCINFO with Arch's makepkg"
  docker build --quiet --platform linux/amd64 --tag hyprlayer-builder/aur scripts/builders/aur >/dev/null
  docker run --rm --platform linux/amd64 --volume "$OUT:/out" hyprlayer-builder/aur
  for d in aur aur-server; do
    [[ -s "$OUT/$d/.SRCINFO" ]] || { echo "makepkg produced no $d/.SRCINFO" >&2; exit 1; }
  done
fi
