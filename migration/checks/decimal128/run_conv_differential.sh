#!/usr/bin/env bash
# Differential for realm_binary64_to_bid128: the vendored C++ against the Rust mirror.
#
# This function cannot be linked (anonymous namespace, inlined at -O3, and the Intel
# library's own binary64_to_bid128 is not in this build at all), so the C++ side includes
# it textually out of decimal128.cpp and the Rust side is the port. Both read the same
# corpus of raw doubles and print `bits w0 w1 flags`.
#
# The flags are compared too. They are an output of this function -- `pfpsf` carries
# invalid/underflow/inexact -- and a driver that printed only the 128-bit value would miss
# every flag bug, which is the base64 `.capacity()` mistake in a new costume.
#
# Every rounding mode is swept. The default (round-to-nearest-even) cannot distinguish the
# sign term in the round-bound index, because rows 0,1 of bid_roundbound_128 are identical
# to rows 2,3; under directed rounding they differ and the control bites. Sweeping the mode
# is what turned that from an untested branch into a tested one.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
HERE="$ROOT/migration/checks/decimal128"
ARCHIVE="$ROOT/build/oracle/realm-core/src/realm/librealm.a"
N_RANDOM="${N_RANDOM:-3000000}"

[ -f "$ARCHIVE" ] || { echo "missing $ARCHIVE -- run 'make oracle' first"; exit 2; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "== extracting the vendored block =="
python3 "$HERE/extract_block.py" "$WORK"

echo "== building the C++ oracle side =="
# librealm.a is on the link line for __bid_IDEC_glbround alone; the conversion code and
# its tables come in textually.
clang++ -std=c++17 -O2 -w -I "$ROOT/upstream/src" -I "$WORK" \
    "$HERE/emit_conv.cpp" "$ARCHIVE" -o "$WORK/oracle" 2>&1 | grep -v '^ld: warning' || true

echo "== building the Rust side =="
# --target x86_64-apple-darwin, not the host default: the shell here runs under Rosetta
# and the oracle is x86_64. A native arm64 build links but compares two different ABIs.
# overflow-checks=on deliberately: the C wraps freely at -O3 -DNDEBUG, so a missed
# wrapping_* in the port should panic loudly rather than diverge quietly.
rustc --edition 2021 -O -C overflow-checks=on --target x86_64-apple-darwin \
    -C link-arg="$ARCHIVE" -C link-arg=-lc++ \
    "$HERE/rust_conv_driver.rs" -o "$WORK/rust" 2>/dev/null

echo "== generating the corpus =="
python3 "$HERE/gen_doubles.py" "$N_RANDOM" > "$WORK/corpus.bin"
COUNT=$(( $(wc -c < "$WORK/corpus.bin") / 8 ))
echo "   $COUNT doubles"

fail=0
for R in 0 1 2 3 4; do
    ROUND=$R "$WORK/oracle" < "$WORK/corpus.bin" > "$WORK/o.txt"
    ROUND=$R "$WORK/rust"   < "$WORK/corpus.bin" > "$WORK/r.txt"
    if cmp -s "$WORK/o.txt" "$WORK/r.txt"; then
        echo "   rounding mode $R: identical on $COUNT doubles"
    else
        echo "   rounding mode $R: DIVERGES on $(diff "$WORK/o.txt" "$WORK/r.txt" | grep -c '^<') lines"
        diff "$WORK/o.txt" "$WORK/r.txt" | head -10
        fail=1
    fi
done

[ "$fail" -eq 0 ] || { echo "FAIL"; exit 1; }
echo "PASS: C++ and Rust agree on $COUNT doubles x 5 rounding modes = $((COUNT * 5)) comparisons."
