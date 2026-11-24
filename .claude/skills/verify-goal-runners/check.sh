#!/bin/sh
# Model-checks GoalRunners.tla one rule at a time and compares with expected.txt.
# Usage: check.sh [SPEC.tla]   (defaults to the GoalRunners.tla beside this script)
# Prints one line per rule, PASS or FAIL, and keeps each TLC log in $OUT (default: a temp dir).
set -eu

here=$(cd "$(dirname "$0")" && pwd)
spec=${1:-$here/GoalRunners.tla}
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

rules=$(sed -n '/^INVARIANTS/,$p' "$here/GoalRunners.cfg" | tail -n +2 | tr -d ' ')
consts=$(sed -n '/^CONSTANTS/,/^SPECIFICATION/p' "$here/GoalRunners.cfg" | sed '$d')

got=$out/result.txt
: > "$got"
for rule in $rules; do
  dir=$out/$rule
  mkdir -p "$dir"
  cp "$spec" "$dir/GoalRunners.tla"
  printf '%s\nSPECIFICATION Spec\nINVARIANT %s\n' "$consts" "$rule" > "$dir/GoalRunners.cfg"
  (cd "$dir" && "$java" -XX:+UseParallelGC -cp "$TLA2TOOLS" tlc2.TLC \
      -workers auto -deadlock -metadir "$dir/states" GoalRunners.tla > tlc.out 2>&1) || true
  if grep -q "Invariant $rule is violated" "$dir/tlc.out"; then
    verdict=FAIL
  elif grep -q "No error has been found" "$dir/tlc.out"; then
    verdict=PASS
  else
    verdict=ERROR
  fi
  printf '%s %s\n' "$verdict" "$rule" | tee -a "$got"
done

echo "logs: $out"
if diff -u "$here/expected.txt" "$got"; then
  echo "matches expected.txt"
else
  echo "differs from expected.txt"
  exit 1
fi
