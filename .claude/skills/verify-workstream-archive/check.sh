#!/bin/sh
# Model-checks ArchiveMove.tla one rule at a time, for each ArchiveMove-<design>.cfg,
# and compares the verdicts with expected.txt.
# Usage: check.sh [SPEC.tla]   (defaults to the ArchiveMove.tla beside this script)
# OUT=dir keeps the TLC logs (default: a temp dir).
set -eu

here=$(cd "$(dirname "$0")" && pwd)
spec=${1:-$here/ArchiveMove.tla}
tlc_pin='http:tla2tools[url=https://github.com/tlaplus/tlaplus/releases/download/v1.7.4/tla2tools.jar,checksum=sha256:936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88]@1.7.4'

if [ -z "${TLA2TOOLS:-}" ]; then
  mise install --quiet "$tlc_pin" >/dev/null
  TLA2TOOLS=$(mise where "$tlc_pin")/tla2tools.jar
fi
java=${JAVA:-java}
# A mise `java` shim with no version set fails; fall back to an installed one.
if [ -z "${JAVA:-}" ] && ! "$java" -version >/dev/null 2>&1; then
  java=$(ls -d "${MISE_DATA_DIR:-$HOME/.local/share/mise}"/installs/java/*/bin/java 2>/dev/null | tail -1)
fi
out=${OUT:-$(mktemp -d)}
mkdir -p "$out"

got=$out/result.txt
: > "$got"
for cfg in "$here"/ArchiveMove-*.cfg; do
  design=$(basename "$cfg" .cfg | sed 's/^ArchiveMove-//')
  rules=$(sed -n '/^INVARIANTS/,$p' "$cfg" | tail -n +2 | tr -d ' ')
  consts=$(sed -n '/^CONSTANTS/,/^SPECIFICATION/p' "$cfg" | sed '$d')
  for rule in $rules; do
    dir=$out/$design/$rule
    mkdir -p "$dir"
    cp "$spec" "$dir/ArchiveMove.tla"
    printf '%s\nSPECIFICATION Spec\nINVARIANT %s\n' "$consts" "$rule" > "$dir/ArchiveMove.cfg"
    (cd "$dir" && "$java" -XX:+UseParallelGC -cp "$TLA2TOOLS" tlc2.TLC \
        -workers auto -deadlock -metadir "$dir/states" ArchiveMove.tla > tlc.out 2>&1) || true
    if grep -q "Invariant $rule is violated" "$dir/tlc.out"; then
      verdict=FAIL
    elif grep -q "No error has been found" "$dir/tlc.out"; then
      verdict=PASS
    else
      verdict=ERROR
    fi
    printf '%s %s %s\n' "$verdict" "$design" "$rule" | tee -a "$got"
  done
done

echo "logs: $out"
if diff -u "$here/expected.txt" "$got"; then
  echo "matches expected.txt"
else
  echo "differs from expected.txt"
  exit 1
fi
