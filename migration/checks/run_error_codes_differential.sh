#!/usr/bin/env bash
# Differential check for error_codes.cpp -- the only real evidence for this unit.
#
# `make verify` does judge this unit: erase_churn and many_commits exercise
# create/insert/erase/get/lower_bound/update_from_parent and pass byte-identical.
# Nothing reaches set, truncate or upper_bound. This covers those, plus every width
# boundary, and compares the raw node bytes.
#
# Shape is "whole-archive + link order" per .claude/rules/evidence-and-linkage.md:
# error_codes.cpp.o's undefined set reaches Node, Allocator and terminate, so
# single-object linking is not an option. Both drivers link all of librealm.a; the
# Rust one puts librealm_core_rs.a first, which is exactly what `make hybrid` does.
#
# Run after `make oracle hybrid`. Exits non-zero on any disagreement.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ORACLE_DIR="$ROOT/build/oracle"
WORK="$ROOT/build/out/error-codes-diff"

HOST_ARCH="$(uname -m)"
case "$HOST_ARCH" in
  arm64)  RUST_TARGET=aarch64-apple-darwin ;;
  x86_64) RUST_TARGET=x86_64-apple-darwin ;;
  *)      RUST_TARGET="$(rustc -vV | sed -n 's/^host: //p')" ;;
esac
RUST_LIB="$ROOT/target/$RUST_TARGET/release/librealm_core_rs.a"
REALM_A="$ORACLE_DIR/realm-core/src/realm/librealm.a"

for f in "$REALM_A" "$RUST_LIB"; do
  [ -f "$f" ] || { echo "missing $f — run 'make oracle hybrid' first" >&2; exit 1; }
done

mkdir -p "$WORK"

# Same flags as the oracle, for the same reason `make hybrid` refuses to build under
# flag drift: a driver compiled differently from the code it links tests something
# other than what ships.
CXXFLAGS=(-std=c++20 -O3 -DNDEBUG -arch "$HOST_ARCH" -Wno-invalid-specialization
          -I "$ROOT/upstream/src" -I "$ORACLE_DIR/realm-core/src")
LIBS=(-framework Foundation -framework Security -lz -lcompression)
SRC="$ROOT/migration/checks/error_codes_differential.cpp"

echo "==> building driver against C++ error_codes"
c++ "${CXXFLAGS[@]}" -o "$WORK/driver_cxx" "$SRC" "$REALM_A" "${LIBS[@]}"

echo "==> building driver against Rust error_codes (staticlib first)"
c++ "${CXXFLAGS[@]}" -o "$WORK/driver_rust" "$SRC" "$RUST_LIB" "$REALM_A" "${LIBS[@]}"

# Guard. Unlike array_unsigned, this unit HAS a file-static to fingerprint:
# `realm::string_to_error_code`, the 160-entry table, is a local symbol emitted only by
# error_codes.cpp.o. Its presence means the C++ object was extracted; its absence means
# the Rust definitions won the link. That is a direct test, not the crate-probe proxy.
if nm "$WORK/driver_rust" | grep -q "string_to_error_code"; then
  echo "FAIL: driver_rust contains the C++ table string_to_error_code." >&2
  echo "      The C++ object was extracted; this comparison would be C++ against C++." >&2
  exit 1
fi
if ! nm "$WORK/driver_cxx" | grep -q "string_to_error_code"; then
  echo "FAIL: driver_cxx does NOT contain string_to_error_code -- wrong object linked." >&2
  exit 1
fi
if ! nm "$WORK/driver_rust" | grep -q "_realm_rs_units_ported"; then
  echo "FAIL: driver_rust does not contain the crate probe; the Rust never linked." >&2
  exit 1
fi

echo "==> running both"
"$WORK/driver_cxx"  > "$WORK/out_cxx.txt"
"$WORK/driver_rust" > "$WORK/out_rust.txt"

lines=$(wc -l < "$WORK/out_cxx.txt" | tr -d ' ')
if [ "$lines" -lt 2000 ]; then
  echo "FAIL: only $lines lines of output — the driver did not run the corpus." >&2
  exit 1
fi

if diff -q "$WORK/out_cxx.txt" "$WORK/out_rust.txt" >/dev/null; then
  echo "PASS: C++ and Rust agree on all $lines probe lines (table rows, categories, from_string search)."
else
  echo "FAIL: C++ and Rust disagree. First differences:" >&2
  diff "$WORK/out_cxx.txt" "$WORK/out_rust.txt" | head -40 >&2
  exit 1
fi
