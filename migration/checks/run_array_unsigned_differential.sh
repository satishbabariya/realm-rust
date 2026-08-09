#!/usr/bin/env bash
# Differential check for array_unsigned.cpp — evidence for the paths no trace reaches.
#
# `make verify` does judge this unit: erase_churn and many_commits exercise
# create/insert/erase/get/lower_bound/update_from_parent and pass byte-identical.
# Nothing reaches set, truncate or upper_bound. This covers those, plus every width
# boundary, and compares the raw node bytes.
#
# Shape is "whole-archive + link order" per .claude/rules/evidence-and-linkage.md:
# array_unsigned.cpp.o's undefined set reaches Node, Allocator and terminate, so
# single-object linking is not an option. Both drivers link all of librealm.a; the
# Rust one puts librealm_core_rs.a first, which is exactly what `make hybrid` does.
#
# Run after `make oracle hybrid`. Exits non-zero on any disagreement.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ORACLE_DIR="$ROOT/build/oracle"
WORK="$ROOT/build/out/array-unsigned-diff"

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
SRC="$ROOT/migration/checks/array_unsigned_differential.cpp"

echo "==> building driver against C++ array_unsigned"
c++ "${CXXFLAGS[@]}" -o "$WORK/driver_cxx" "$SRC" "$REALM_A" "${LIBS[@]}"

echo "==> building driver against Rust array_unsigned (staticlib first)"
c++ "${CXXFLAGS[@]}" -o "$WORK/driver_rust" "$SRC" "$RUST_LIB" "$REALM_A" "${LIBS[@]}"

# Guard. array_unsigned.cpp has no file-static to fingerprint — everything it defines
# besides the ten methods is weak and supplied by other objects — so per
# evidence-and-linkage.md we fall back to the crate probe: present in the Rust driver,
# absent from the C++ one. Without this the comparison could be C++ against C++ and
# would pass while proving nothing.
if ! nm "$WORK/driver_rust" | grep -q "_realm_rs_units_ported"; then
  echo "FAIL: driver_rust does not contain the crate probe; the Rust never linked." >&2
  exit 1
fi
if nm "$WORK/driver_cxx" | grep -q "_realm_rs_units_ported"; then
  echo "FAIL: driver_cxx contains the crate probe; it is not a pure C++ build." >&2
  exit 1
fi

# And check the implementations really differ in origin: the Rust driver's
# ArrayUnsigned symbols must sit in the same object as the probe (codegen-units = 1),
# i.e. within a few KB of it. In the C++ driver they must not.
probe_addr=$(nm "$WORK/driver_rust" | awk '/_realm_rs_units_ported$/{print $1}')
au_addr=$(nm "$WORK/driver_rust" | awk '/ArrayUnsigned9set_widthEh$/{print $1}')
if [ -z "$probe_addr" ] || [ -z "$au_addr" ]; then
  echo "FAIL: could not locate probe or ArrayUnsigned::set_width in driver_rust." >&2
  exit 1
fi
delta=$(( 0x$probe_addr > 0x$au_addr ? 0x$probe_addr - 0x$au_addr : 0x$au_addr - 0x$probe_addr ))
if [ "$delta" -gt 65536 ]; then
  echo "FAIL: ArrayUnsigned::set_width is ${delta} bytes from the crate probe in" >&2
  echo "      driver_rust. With codegen-units = 1 they belong to the same object;" >&2
  echo "      this distance means the C++ definition won the link." >&2
  exit 1
fi

echo "==> running both"
"$WORK/driver_cxx"  > "$WORK/out_cxx.txt"
"$WORK/driver_rust" > "$WORK/out_rust.txt"

lines=$(wc -l < "$WORK/out_cxx.txt" | tr -d ' ')
if [ "$lines" -lt 200 ]; then
  echo "FAIL: only $lines lines of output — the driver did not run the corpus." >&2
  exit 1
fi

if diff -q "$WORK/out_cxx.txt" "$WORK/out_rust.txt" >/dev/null; then
  echo "PASS: C++ and Rust agree on all $lines probe lines (node bytes, widths, bounds)."
else
  echo "FAIL: C++ and Rust disagree. First differences:" >&2
  diff "$WORK/out_cxx.txt" "$WORK/out_rust.txt" | head -40 >&2
  exit 1
fi
