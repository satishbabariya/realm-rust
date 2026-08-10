#!/usr/bin/env bash
# Can Rust throw a realm::LogicError that C++ catches?
#
# This settles what six park files assumed. It builds the SAME Rust source twice, once
# with panic=abort (what the workspace sets today) and once with panic=unwind, and runs
# both against a C++ driver that catches realm::LogicError by reference.
#
# Expected: abort fails, unwind passes. If that ever flips, the exception verdicts in
# migration/blocked/ need revisiting.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
HERE="$ROOT/migration/checks/throw_probe"
REALM_A="$ROOT/build/oracle/realm-core/src/realm/librealm.a"
[ -f "$REALM_A" ] || { echo "missing $REALM_A -- run 'make oracle' first" >&2; exit 2; }
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
LIBS=(-framework Foundation -framework Security -lz -lcompression)

for mode in abort unwind; do
    rustc --edition 2021 -O --crate-type staticlib --target x86_64-apple-darwin \
        -C panic=$mode "$HERE/probe.rs" -o "$WORK/libprobe_$mode.a" 2>/dev/null
    clang++ -std=c++20 -O2 -w -I "$ROOT/upstream/src" -I "$ROOT/build/oracle/realm-core/src" \
        "$HERE/driver.cpp" "$WORK/libprobe_$mode.a" "$REALM_A" "${LIBS[@]}" \
        -o "$WORK/probe_$mode" 2>&1 | grep -v '^ld: warning' || true
    out="$("$WORK/probe_$mode" 2>&1)"; rc=$?
    printf "  panic=%-6s exit=%-3d %s\n" "$mode" "$rc" "$out"
    if [ "$mode" = abort ] && [ "$rc" -eq 0 ]; then
        echo "UNEXPECTED: panic=abort let the unwind through. Re-check the park files." >&2; exit 1
    fi
    if [ "$mode" = unwind ] && [ "$rc" -ne 0 ]; then
        echo "UNEXPECTED: panic=unwind failed to propagate. Re-check the park files." >&2; exit 1
    fi
done
echo "PASS: throwing from Rust works under panic=unwind and aborts under panic=abort."
