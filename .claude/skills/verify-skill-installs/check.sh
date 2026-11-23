#!/bin/sh
# Model-checks SkillInstalls.tla one rule at a time, for each SkillInstalls-<platform>.cfg,
# and compares the verdicts with expected.txt.
# Usage: check.sh [SPEC.tla]   (defaults to the SkillInstalls.tla beside this script)
# OUT=dir keeps the TLC logs (default: a temp dir).
set -eu

here=$(cd "$(dirname "$0")" && pwd)
spec=${1:-$here/SkillInstalls.tla}
tlc_pin='http:tla2tools[url=https://github.com/tlaplus/tlaplus/releases/download/v1.7.4/tla2tools.jar,checksum=sha256:936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88]@1.7.4'

if [ -z "${TLA2TOOLS:-}" ]; then
  mise install --quiet "$tlc_pin" >/dev/null
  TLA2TOOLS=$(mise where "$tlc_pin")/tla2tools.jar
fi
java=${JAVA:-java}
out=${OUT:-$(mktemp -d)}
mkdir -p "$out"

got=$out/result.txt
: > "$got"
for cfg in "$here"/SkillInstalls-*.cfg; do
  platform=$(basename "$cfg" .cfg | sed 's/^SkillInstalls-//')
  rules=$(sed -n '/^INVARIANTS/,$p' "$cfg" | tail -n +2 | tr -d ' ')
  consts=$(sed -n '/^CONSTANTS/,/^SPECIFICATION/p' "$cfg" | sed '$d')
  for rule in $rules; do
    dir=$out/$platform/$rule
    mkdir -p "$dir"
    cp "$spec" "$dir/SkillInstalls.tla"
    printf '%s\nSPECIFICATION Spec\nINVARIANT %s\n' "$consts" "$rule" > "$dir/SkillInstalls.cfg"
    (cd "$dir" && "$java" -XX:+UseParallelGC -cp "$TLA2TOOLS" tlc2.TLC \
        -workers auto -deadlock -metadir "$dir/states" SkillInstalls.tla > tlc.out 2>&1) || true
    if grep -q "Invariant $rule is violated" "$dir/tlc.out"; then
      verdict=FAIL
    elif grep -q "No error has been found" "$dir/tlc.out"; then
      verdict=PASS
    else
      verdict=ERROR
    fi
    printf '%s %s %s\n' "$verdict" "$platform" "$rule" | tee -a "$got"
  done
done

echo "logs: $out"
if diff -u "$here/expected.txt" "$got"; then
  echo "matches expected.txt"
else
  echo "differs from expected.txt"
  exit 1
fi
