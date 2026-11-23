#!/bin/sh
# Model-checks SessionTurns.tla, RestartWake.tla and SessionSync.tla one rule at a time and compares with expected.txt.
# Usage: check.sh [Model ...]   SRC=dir checks the .tla and .cfg files in dir instead (a break check).
# Prints one line per rule, PASS or FAIL, and keeps each TLC log in $OUT (default: a temp dir).
set -eu

here=$(cd "$(dirname "$0")" && pwd)
src=${SRC:-$here}
tlc_pin='http:tla2tools[url=https://github.com/tlaplus/tlaplus/releases/download/v1.7.4/tla2tools.jar,checksum=sha256:936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88]@1.7.4'

if [ -z "${TLA2TOOLS:-}" ]; then
  mise install --quiet "$tlc_pin" >/dev/null
  TLA2TOOLS=$(mise where "$tlc_pin")/tla2tools.jar
fi
java=${JAVA:-java}
workers=${WORKERS:-4}
out=${OUT:-$(mktemp -d)}
models=${*:-SessionTurns RestartWake SessionSync}
mkdir -p "$out"

got=$out/result.txt
: > "$got"
for model in $models; do
  for cfg in "$src/$model".cfg "$src/$model"-*.cfg; do
    [ -f "$cfg" ] || continue
    name=$(basename "$cfg" .cfg)
    rules=$(sed -n '/^INVARIANTS/,$p' "$cfg" | tail -n +2 | tr -d ' ')
    consts=$(sed -n '/^CONSTANTS/,/^SPECIFICATION/p' "$cfg" | sed '$d')
    for rule in $rules; do
      dir=$out/$name.$rule
      mkdir -p "$dir"
      cp "$src/$model.tla" "$dir/$model.tla"
      printf '%s\nSPECIFICATION Spec\nINVARIANT %s\n' "$consts" "$rule" > "$dir/$model.cfg"
      (cd "$dir" && "$java" -XX:+UseParallelGC -cp "$TLA2TOOLS" tlc2.TLC \
          -workers "$workers" -metadir "$dir/states" "$model.tla" > tlc.out 2>&1) || true
      rm -rf "$dir/states"
      if grep -q "Invariant $rule is violated" "$dir/tlc.out"; then
        verdict=FAIL
      elif grep -q "No error has been found" "$dir/tlc.out"; then
        verdict=PASS
      else
        verdict=ERROR
      fi
      printf '%s %s\n' "$verdict" "$name.$rule" | tee -a "$got"
    done
  done
done

echo "logs: $out"
if [ $# -gt 0 ]; then
  exit 0
fi
if diff -u "$here/expected.txt" "$got"; then
  echo "matches expected.txt"
else
  echo "differs from expected.txt"
  exit 1
fi
