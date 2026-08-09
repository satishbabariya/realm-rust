#!/usr/bin/env bash
# Differential check for array_blobs_small.cpp -- the only real evidence for this unit.
#
# `make verify` does judge this unit: erase_churn and many_commits exercise
# create/insert/erase/get/lower_bound/update_from_parent and pass byte-identical.
# Nothing reaches set, truncate or upper_bound. This covers those, plus every width
# boundary, and compares the raw node bytes.
#
# Shape is "whole-archive + link order" per .claude/rules/evidence-and-linkage.md:
# array_blobs_small.cpp.o's undefined set reaches Node, Allocator and terminate, so
# single-object linking is not an option. Both drivers link all of librealm.a; the
# Rust one puts librealm_core_rs.a first, which is exactly what `make hybrid` does.
#
# Run after `make oracle hybrid`. Exits non-zero on any disagreement.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ORACLE_DIR="$ROOT/build/oracle"
WORK="$ROOT/build/out/array-blobs-small-diff"

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
SRC="$ROOT/migration/checks/array_blobs_small_differential.cpp"

echo "==> building driver against C++ array_blobs_small"
c++ "${CXXFLAGS[@]}" -o "$WORK/driver_cxx" "$SRC" "$REALM_A" "${LIBS[@]}"

echo "==> building driver against Rust array_blobs_small (staticlib first)"
c++ "${CXXFLAGS[@]}" -o "$WORK/driver_rust" "$SRC" "$RUST_LIB" "$REALM_A" "${LIBS[@]}"

nm "$WORK/driver_rust" > "$WORK/syms_rust.txt"
nm "$WORK/driver_cxx"  > "$WORK/syms_cxx.txt"
# Guard. array_blobs_small.cpp has no unique file-static and its ZT* are coalesced, so fall
# back to the crate probe plus an address-adjacency check: with codegen-units = 1 the
# Rust definitions live in the same object as the probe.
if ! grep -q "_realm_rs_units_ported" "$WORK/syms_rust.txt"; then
  echo "FAIL: driver_rust does not contain the crate probe; the Rust never linked." >&2
  exit 1
fi
if grep -q "_realm_rs_units_ported" "$WORK/syms_cxx.txt"; then
  echo "FAIL: driver_cxx contains the crate probe; it is not a pure C++ build." >&2
  exit 1
fi
probe=$(awk '/_realm_rs_units_ported$/{print $1}' "$WORK/syms_rust.txt")
repl=$(awk '/ArraySmallBlobs6insertEmNS_10BinaryDataEb$/{print $1; exit}' "$WORK/syms_rust.txt")
if [ -z "$repl" ]; then echo "FAIL: ArraySmallBlobs::insert missing from driver_rust." >&2; exit 1; fi
d=$(( 0x$probe > 0x$repl ? 0x$probe - 0x$repl : 0x$repl - 0x$probe ))
if [ "$d" -gt 65536 ]; then
  echo "FAIL: ArraySmallBlobs::insert is $d bytes from the crate probe; with codegen-units=1" >&2
  echo "      they belong to the same object, so the C++ definition won the link." >&2
  exit 1
fi

echo "==> running both"
"$WORK/driver_cxx"  > "$WORK/out_cxx.txt"
"$WORK/driver_rust" > "$WORK/out_rust.txt"

lines=$(wc -l < "$WORK/out_cxx.txt" | tr -d ' ')
# Completeness guard: the driver's last line is a literal terminator. That is a direct
# check that it ran to the end, unlike a line-count threshold which has to be re-tuned
# whenever the corpus changes and silently passes a driver that died one case early.
for f in out_cxx out_rust; do
  if [ "$(tail -1 "$WORK/$f.txt")" != "done" ]; then
    echo "FAIL: $f.txt does not end with the 'done' terminator -- the driver aborted." >&2
    tail -3 "$WORK/$f.txt" >&2
    exit 1
  fi
done

if diff -q "$WORK/out_cxx.txt" "$WORK/out_rust.txt" >/dev/null; then
  echo "PASS: C++ and Rust agree on all $lines probe lines (mid-list insert/erase/set, find_first, legacy reads)."
else
  echo "FAIL: C++ and Rust disagree. First differences:" >&2
  diff "$WORK/out_cxx.txt" "$WORK/out_rust.txt" | head -40 >&2
  exit 1
fi
