#!/bin/sh
# Model-checks StreamCredit.tla and EventGaps.tla one rule at a time and compares with expected.txt.
# Usage: check.sh   Prints one line per rule, PASS or FAIL, and keeps each TLC log in $OUT (default: a temp dir).
set -eu

here=$(cd "$(dirname "$0")" && pwd)
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
workers=${WORKERS:-4}
out=${OUT:-$(mktemp -d)}
mkdir -p "$out"

got=$out/result.txt
: > "$got"
for model in StreamCredit EventGaps; do
  rules=$(sed -n '/^INVARIANTS/,$p' "$here/$model.cfg" | tail -n +2 | tr -d ' ')
  consts=$(sed -n '/^CONSTANTS/,/^SPECIFICATION/p' "$here/$model.cfg" | sed '$d')
  for rule in $rules; do
    dir=$out/$model.$rule
    mkdir -p "$dir"
    cp "$here/$model.tla" "$dir/$model.tla"
    printf '%s\nSPECIFICATION Spec\nINVARIANT %s\n' "$consts" "$rule" > "$dir/$model.cfg"
    (cd "$dir" && "$java" -XX:+UseParallelGC -cp "$TLA2TOOLS" tlc2.TLC \
        -workers "$workers" -metadir "$dir/states" "$model.tla" > tlc.out 2>&1) || true
    if grep -q "Invariant $rule is violated" "$dir/tlc.out"; then
      verdict=FAIL
    elif grep -q "No error has been found" "$dir/tlc.out"; then
      verdict=PASS
    else
      verdict=ERROR
    fi
    printf '%s %s\n' "$verdict" "$model.$rule" | tee -a "$got"
  done
done

echo "logs: $out"
if diff -u "$here/expected.txt" "$got"; then
  echo "matches expected.txt"
else
  echo "differs from expected.txt"
  exit 1
fi
