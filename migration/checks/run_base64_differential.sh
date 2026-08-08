#!/usr/bin/env bash
# Differential check for util/base64.cpp — the evidence `make diff-test` cannot give.
#
# base64 never reaches a .realm file, so byte-identity of the output files says
# nothing about whether this unit was ported correctly. This script builds the same
# driver twice — once against the oracle's C++ object, once against the Rust
# staticlib — and requires the two dumps to be byte-identical.
#
# Run after `make oracle hybrid`. Exits non-zero on any disagreement.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ORACLE_DIR="$ROOT/build/oracle"
HYBRID_DIR="$ROOT/build/hybrid"
WORK="$ROOT/build/out/base64-diff"

CXX_OBJ="$ORACLE_DIR/realm-core/src/realm/CMakeFiles/Storage.dir/util/base64.cpp.o"
HOST_ARCH="$(uname -m)"
case "$HOST_ARCH" in
  arm64)  RUST_TARGET=aarch64-apple-darwin ;;
  x86_64) RUST_TARGET=x86_64-apple-darwin ;;
  *)      RUST_TARGET="$(rustc -vV | sed -n 's/^host: //p')" ;;
esac
RUST_LIB="$ROOT/target/$RUST_TARGET/release/librealm_core_rs.a"

for f in "$CXX_OBJ" "$RUST_LIB"; do
  if [ ! -f "$f" ]; then
    echo "missing $f — run 'make oracle hybrid' first" >&2
    exit 1
  fi
done

mkdir -p "$WORK"

# The build flags must match the oracle's, for the same reason `make hybrid` refuses
# to build under flag drift: a driver compiled differently from the object it links
# is testing something other than what ships.
CXXFLAGS=(-std=c++20 -O3 -DNDEBUG -arch "$HOST_ARCH" -Wno-invalid-specialization
          -I "$ROOT/upstream/src" -I "$ORACLE_DIR/realm-core/src")

echo "==> building driver against the C++ oracle object"
c++ "${CXXFLAGS[@]}" -o "$WORK/driver_cxx" "$ROOT/migration/checks/base64_differential.cpp" "$CXX_OBJ"

echo "==> building driver against the Rust staticlib"
c++ "${CXXFLAGS[@]}" -o "$WORK/driver_rust" "$ROOT/migration/checks/base64_differential.cpp" "$RUST_LIB" \
    -framework Foundation -framework Security -lz -lcompression

# Guard: if the Rust staticlib somehow failed to supply the symbols and they came
# from somewhere else, this whole comparison is vacuous. The C++ TU's file-static
# tables are the fingerprint — their presence means base64.cpp.o got linked in.
if nm "$WORK/driver_rust" | grep -q "g_base64_encoding_chars"; then
  echo "FAIL: driver_rust contains the C++ translation unit's static tables." >&2
  echo "      The Rust definitions did not win the link; this comparison would be" >&2
  echo "      C++ against C++ and would pass while proving nothing." >&2
  exit 1
fi
if ! nm "$WORK/driver_cxx" | grep -q "g_base64_encoding_chars"; then
  echo "FAIL: driver_cxx does NOT contain the C++ static tables — wrong object linked." >&2
  exit 1
fi

echo "==> running both"
"$WORK/driver_cxx" > "$WORK/out_cxx.txt"
"$WORK/driver_rust" > "$WORK/out_rust.txt"

lines=$(wc -l < "$WORK/out_cxx.txt" | tr -d ' ')
if [ "$lines" -lt 1000 ]; then
  echo "FAIL: only $lines lines of output — the driver did not run the corpus." >&2
  exit 1
fi

if diff -q "$WORK/out_cxx.txt" "$WORK/out_rust.txt" >/dev/null; then
  echo "PASS: C++ and Rust agree on all $lines probe lines."
else
  echo "FAIL: C++ and Rust disagree. First differences:" >&2
  diff "$WORK/out_cxx.txt" "$WORK/out_rust.txt" | head -40 >&2
  exit 1
fi

# The hybrid binary itself must also be free of the C++ TU, not just this driver.
if [ -x "$HYBRID_DIR/trace_runner" ]; then
  if nm "$HYBRID_DIR/trace_runner" | grep -q "g_base64_encoding_chars"; then
    echo "FAIL: build/hybrid/trace_runner still contains the C++ base64 tables." >&2
    echo "      The hybrid is running C++ base64, not the Rust port." >&2
    exit 1
  fi
  echo "PASS: build/hybrid/trace_runner carries no C++ base64 translation unit."
fi
