#!/usr/bin/env bash
# Differential check for decimal128.cpp -- the methods.
#
# migration/checks/decimal128/run_conv_differential.sh covers the other half, the
# reimplemented realm_binary64_to_bid128, over 3M doubles at every rounding mode. This
# covers the 47 symbols that wrap linkable BID entry points, and the Decimal128(double)
# constructor end to end including its two bid128_quantize retries.
#
# The unit is byte-visible and UNTRACED: Decimal128 is a stored column type but the trace
# schema is int/string/double/bool, so no trace stores a decimal. `make verify` proves the
# link is intact and nothing else regressed. This is the evidence for the unit itself.
#
# Shape is "whole-archive + link order" per .claude/rules/evidence-and-linkage.md. Both
# drivers link all of librealm.a; the Rust one puts librealm_core_rs.a first, which is what
# `make hybrid` does.
#
# Run after `make oracle hybrid`. Exits non-zero on any disagreement.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ORACLE_DIR="$ROOT/build/oracle"
WORK="$ROOT/build/out/decimal128-diff"

HOST_ARCH="$(uname -m)"
case "$HOST_ARCH" in
  arm64)  RUST_TARGET=aarch64-apple-darwin ;;
  x86_64) RUST_TARGET=x86_64-apple-darwin ;;
  *)      RUST_TARGET="$(rustc -vV | sed -n 's/^host: //p')" ;;
esac
RUST_LIB="$ROOT/target/$RUST_TARGET/release/librealm_core_rs.a"
REALM_A="$ORACLE_DIR/realm-core/src/realm/librealm.a"

for f in "$REALM_A" "$RUST_LIB"; do
  [ -f "$f" ] || { echo "missing $f -- run 'make oracle hybrid' first" >&2; exit 1; }
done

mkdir -p "$WORK"

CXXFLAGS=(-std=c++20 -O3 -DNDEBUG -arch "$HOST_ARCH" -Wno-invalid-specialization
          -I "$ROOT/upstream/src" -I "$ORACLE_DIR/realm-core/src")
LIBS=(-framework Foundation -framework Security -lz -lcompression)
SRC="$ROOT/migration/checks/decimal128_differential.cpp"

echo "==> building driver against C++ decimal128"
c++ "${CXXFLAGS[@]}" -o "$WORK/driver_cxx" "$SRC" "$REALM_A" "${LIBS[@]}"

echo "==> building driver against Rust decimal128 (staticlib first)"
c++ "${CXXFLAGS[@]}" -o "$WORK/driver_rust" "$SRC" "$RUST_LIB" "$REALM_A" "${LIBS[@]}"

nm "$WORK/driver_rust" > "$WORK/syms_rust.txt"
nm "$WORK/driver_cxx"  > "$WORK/syms_cxx.txt"

# Guard. Unlike array_timestamp, this unit HAS unique file-statics to fingerprint: the
# vendored Intel tables in its anonymous namespace. If decimal128.cpp.o was extracted into
# the Rust driver, they come with it -- which is a direct check that the C++ TU lost the
# link, not an inference from symbol addresses.
FINGERPRINT='_GLOBAL__N_1(14bid_power_five|22bid_coefflimits_bid128)'
if ! grep -Eq "$FINGERPRINT" "$WORK/syms_cxx.txt"; then
  echo "FAIL: driver_cxx lacks decimal128.cpp's private tables; it did not link the C++ TU." >&2
  exit 1
fi
if grep -Eq "$FINGERPRINT" "$WORK/syms_rust.txt"; then
  echo "FAIL: driver_rust contains decimal128.cpp's private tables -- the C++ TU was" >&2
  echo "      extracted, so this compares the C++ against itself." >&2
  exit 1
fi
if ! grep -q "_realm_rs_units_ported" "$WORK/syms_rust.txt"; then
  echo "FAIL: driver_rust does not contain the crate probe; the Rust never linked." >&2
  exit 1
fi
if grep -q "_realm_rs_units_ported" "$WORK/syms_cxx.txt"; then
  echo "FAIL: driver_cxx contains the crate probe; it is not a pure C++ build." >&2
  exit 1
fi

echo "==> running both"
"$WORK/driver_cxx"  > "$WORK/out_cxx.txt"
"$WORK/driver_rust" > "$WORK/out_rust.txt"

lines=$(wc -l < "$WORK/out_cxx.txt" | tr -d ' ')
for f in out_cxx out_rust; do
  if [ "$(tail -1 "$WORK/$f.txt")" != "done" ]; then
    echo "FAIL: $f.txt does not end with the 'done' terminator -- the driver aborted." >&2
    tail -3 "$WORK/$f.txt" >&2
    exit 1
  fi
done

if diff -q "$WORK/out_cxx.txt" "$WORK/out_rust.txt" >/dev/null; then
  echo "PASS: C++ and Rust agree on all $lines probe lines (raw encodings, string capacity, all comparators)."
else
  echo "FAIL: C++ and Rust disagree. First differences:" >&2
  diff "$WORK/out_cxx.txt" "$WORK/out_rust.txt" | head -40 >&2
  exit 1
fi
