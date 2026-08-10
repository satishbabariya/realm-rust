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
# --- second half: a Rust panic must still be fatal under panic=unwind ---
# The workspace gave up panic="abort" to let C++ exceptions through, and installs a panic
# hook to keep Rust bugs loud. If this stops aborting, an overflow trap or index panic in
# the hybrid can be swallowed by a `catch (...)` in C++ and execution continues with
# corrupt state -- the exact failure this project exists to catch.
rustc --edition 2021 -O --crate-type staticlib --target x86_64-apple-darwin \
    -C panic=unwind -C overflow-checks=on "$HERE/panic_probe.rs" -o "$WORK/libpanic.a" 2>/dev/null
clang++ -std=c++20 -O2 -w "$HERE/panic_driver.cpp" "$WORK/libpanic.a" -o "$WORK/panicprobe" \
    2>&1 | grep -v '^ld: warning' || true
out="$("$WORK/panicprobe" 2>&1)"; rc=$?
printf "  panic hook   exit=%-3d %s\n" "$rc" "$(printf '%s' "$out" | tail -1)"
if [ "$rc" -eq 0 ]; then
    echo "FAIL: a Rust panic was not fatal -- C++ may have swallowed it." >&2; exit 1
fi
if [ "$rc" -ne 134 ]; then
    echo "FAIL: expected SIGABRT (134) from the panic hook, got $rc." >&2; exit 1
fi

# --- third: every exception type util/file_mapper throws, caught AS ITS OWN TYPE ---
rustc --edition 2021 -O --crate-type staticlib --target x86_64-apple-darwin -C panic=unwind \
    "$HERE/types_probe.rs" -o "$WORK/libtypes.a" 2>/dev/null
clang++ -std=c++20 -O2 -w -I "$ROOT/upstream/src" -I "$ROOT/build/oracle/realm-core/src" \
    "$HERE/types_driver.cpp" "$WORK/libtypes.a" "$REALM_A" "${LIBS[@]}" -o "$WORK/types" \
    2>&1 | grep -v '^ld: warning' || true
"$WORK/types"; rc=$?
if [ "$rc" -ne 0 ]; then
    echo "FAIL: not every exception type round-tripped as its own type." >&2; exit 1
fi

# --- fourth: does a Rust Drop run when a C++ exception unwinds through it? ---
# This is what makes RAII cleanup (ScopeExitFail and friends) reproducible without a
# catch, which Rust does not have. util/file_mapper::mmap depends on it.
rustc --edition 2021 -O --crate-type staticlib --target x86_64-apple-darwin -C panic=unwind \
    "$HERE/drop_probe.rs" -o "$WORK/libdrop.a" 2>/dev/null
clang++ -std=c++20 -O2 -w -I "$ROOT/upstream/src" -I "$ROOT/build/oracle/realm-core/src" \
    "$HERE/drop_driver.cpp" "$WORK/libdrop.a" "$REALM_A" "${LIBS[@]}" -o "$WORK/drop" \
    2>&1 | grep -v '^ld: warning' || true
"$WORK/drop"; rc=$?
if [ "$rc" -ne 0 ]; then
    echo "FAIL: Rust drops do not run for a foreign unwind -- RAII cleanup is not reproducible." >&2
    exit 1
fi

echo "PASS: Rust throws propagate under panic=unwind, abort under panic=abort,"
echo "      a Rust panic is still fatal (SIGABRT) rather than catchable by C++,"
echo "      all four of util/file_mapper's exception types round-trip by exact type,"
echo "      and Rust drops run for a foreign unwind, so RAII cleanup is reproducible."
