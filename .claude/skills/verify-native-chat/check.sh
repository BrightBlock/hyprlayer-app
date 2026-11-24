#!/bin/sh
# Model-checks SessionTurns.tla (claude), CodexTurns.tla, OpenCodeTurns.tla, RestartWake.tla and SessionSync.tla one rule at a time and compares with expected.txt.
# Usage: check.sh [Model ...]   SRC=dir checks the .tla and .cfg files in dir instead (a break check).
# Prints one line per rule, PASS or FAIL, and keeps the TLC logs in $OUT (default: a temp dir):
# $OUT/<Model>.<Rule>/tlc.out for each FAIL, $OUT/<Model>.pass/tlc.out for the run every PASS shares.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
src=${SRC:-$here}
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
models=${*:-SessionTurns CodexTurns OpenCodeTurns RestartWake SessionSync}
mkdir -p "$out"

got=$out/result.txt
: > "$got"
# One TLC run checks every rule still unresolved. Breadth-first, it stops at the shortest
# path that breaks any of them; that rule is FAIL, it is dropped, and TLC runs again. A run
# that finishes clean passes every rule left.
for model in $models; do
  for cfg in "$src/$model".cfg "$src/$model"-*.cfg; do
    [ -f "$cfg" ] || continue
    name=$(basename "$cfg" .cfg)
    all=$(sed -n '/^INVARIANTS/,$p' "$cfg" | tail -n +2 | tr -d ' ')
    consts=$(sed -n '/^CONSTANTS/,/^SPECIFICATION/p' "$cfg" | sed '$d')
    left=$all
    verdicts=$out/$name.verdicts
    : > "$verdicts"
    while [ -n "$left" ]; do
      dir=$out/$name.run
      rm -rf "$dir"
      mkdir -p "$dir"
      cp "$src/$model.tla" "$dir/$model.tla"
      { printf '%s\nSPECIFICATION Spec\nINVARIANTS\n' "$consts"; printf '  %s\n' $left; } > "$dir/$model.cfg"
      (cd "$dir" && "$java" -XX:+UseParallelGC -cp "$TLA2TOOLS" tlc2.TLC \
          -workers "$workers" -metadir "$dir/states" "$model.tla" > tlc.out 2>&1) || true
      rm -rf "$dir/states"
      broke=$(sed -n 's/^Error: Invariant \([A-Za-z0-9_]*\) is violated.*/\1/p' "$dir/tlc.out" | head -1)
      if [ -n "$broke" ]; then
        rm -rf "$out/$name.$broke"
        mv "$dir" "$out/$name.$broke"
        echo "FAIL $broke" >> "$verdicts"
        left=$(printf '%s\n' $left | grep -vx "$broke" || true)
      elif grep -q "No error has been found" "$dir/tlc.out"; then
        rm -rf "$out/$name.pass"
        mv "$dir" "$out/$name.pass"
        for rule in $left; do echo "PASS $rule" >> "$verdicts"; done
        left=
      else
        rm -rf "$out/$name.error"
        mv "$dir" "$out/$name.error"
        for rule in $left; do echo "ERROR $rule" >> "$verdicts"; done
        left=
      fi
    done
    for rule in $all; do
      verdict=$(grep " $rule\$" "$verdicts" | cut -d' ' -f1)
      printf '%s %s\n' "$verdict" "$name.$rule" | tee -a "$got"
    done
  done
done

echo "logs: $out"
if [ $# -gt 0 ]; then
  exit 0
fi
if sed 's/[[:space:]]*#.*//' "$here/expected.txt" | diff -u - "$got"; then
  echo "matches expected.txt"
else
  echo "differs from expected.txt"
  exit 1
fi
