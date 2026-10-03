#!/usr/bin/env bash
# run.sh - run docs examples with the release binary and record their output beside them.
#
#   bash docs/examples/run.sh              # every example
#   bash docs/examples/run.sh weather ask  # only these (names without .settle)
#
# Each example runs twice in a scratch copy of this folder (so files it writes never land here), from inside
# that copy, which is what `cd docs/examples && settle NAME.settle` does. A program that exits 0 gets NAME.out
# (its printed lines); one that fails gets NAME.err (its error message without the `settle: ` prefix).
# Timings are replaced with <time>, the same rule tests/docs_examples.rs applies. If the two runs differ,
# the example is not deterministic and the script says so.
# The test that checks these files: cargo test --release --test docs_examples
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$HERE/../../target/release/settle"
[[ -x "$BIN" ]] || { echo "run.sh: build first: cargo build --release"; exit 2; }

mask() { sed -E 's/(^|[^0-9.])([0-9]+\.[0-9]+)( ms| frames\/s| s fitting)/\1<time>\3/g; s/(^|[^0-9.])[0-9]+\.[0-9]+s;/\1<time>s;/g; s/(^|[^0-9.])[0-9]+\.[0-9]+s$/\1<time>s/'; }

TMP="$(mktemp -d "${TMPDIR:-/tmp}/settle-docs-run.XXXXXX")"
cp -R "$HERE/." "$TMP/"

if [[ $# -gt 0 ]]; then names=("$@"); else names=(); for f in "$HERE"/*.settle; do names+=("$(basename "$f" .settle)"); done; fi

status=0
for n in "${names[@]}"; do
  [[ -f "$TMP/$n.settle" ]] || { echo "  $n: no such example"; status=1; continue; }
  runs=()
  for k in 1 2; do
    t0=$(date +%s)
    out=$(cd "$TMP" && "$BIN" "$n.settle" 2>"$TMP/.stderr"); rc=$?
    secs=$(( $(date +%s) - t0 ))
    if [[ $rc -eq 0 ]]; then runs+=("OUT$(printf '%s\n' "$out" | mask)"); else runs+=("ERR$(sed -E 's/^settle: //' "$TMP/.stderr" | mask)"); fi
  done
  if [[ "${runs[0]}" != "${runs[1]}" ]]; then echo "  $n: NOT DETERMINISTIC (two runs differ)"; status=1; fi
  r="${runs[0]}"
  if [[ "$r" == OUT* ]]; then
    printf '%s\n' "${r#OUT}" > "$HERE/$n.out"; rm -f "$HERE/$n.err"; echo "  $n: ok (${secs}s) -> $n.out"
  else
    printf '%s\n' "${r#ERR}" > "$HERE/$n.err"; rm -f "$HERE/$n.out"; echo "  $n: error (${secs}s) -> $n.err"
  fi
done
exit $status
