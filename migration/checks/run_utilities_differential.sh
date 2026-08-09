#!/usr/bin/env bash
# Differential check for utilities.cpp.
#
# Covers what diff-test cannot see: the two exported *data* symbols (sse_support /
# avx_support, read by cpu_sse<>() inlined across many TUs), platform_timegm's
# deliberate 32-bit truncation past 2038, FastRand::operator() whose `this` is a bare
# uint64_t, and fastrand's shared-state sequence including the UINT64_MAX modulus.
#
# Run after `make oracle hybrid`. Exits non-zero on any disagreement.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ORACLE_DIR="$ROOT/build/oracle"
HYBRID_DIR="$ROOT/build/hybrid"
WORK="$ROOT/build/out/utilities-diff"

CXX_OBJ="$ORACLE_DIR/realm-core/src/realm/CMakeFiles/Storage.dir/utilities.cpp.o"
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

# Flags must match the oracle's, for the same reason `make hybrid` refuses to build
# under flag drift: a driver compiled differently from the object it links is testing
# something other than what ships.
CXXFLAGS=(-std=c++20 -O3 -DNDEBUG -arch "$HOST_ARCH" -Wno-invalid-specialization
          -I "$ROOT/upstream/src" -I "$ORACLE_DIR/realm-core/src")

echo "==> building driver against the C++ oracle object"
# utilities.cpp pulls in util::Mutex, which pulls in terminate/backtrace, so linking
# the single object is not viable here. Both drivers link the whole realm archive
# instead, and the Rust driver puts its staticlib *first* -- the same link-order
# mechanism `make hybrid` relies on, where the Rust definitions resolve the symbols and
# the C++ object is therefore never extracted from the archive.
REALM_LIB="$ORACLE_DIR/realm-core/src/realm/librealm.a"
if [ ! -f "$REALM_LIB" ]; then
  echo "missing $REALM_LIB -- run 'make oracle' first" >&2
  exit 1
fi
FRAMEWORKS=(-framework Foundation -framework Security -lz -lcompression)
c++ "${CXXFLAGS[@]}" -o "$WORK/driver_cxx" "$ROOT/migration/checks/utilities_differential.cpp" \
    "$REALM_LIB" "${FRAMEWORKS[@]}"

echo "==> building driver against the Rust staticlib"
c++ "${CXXFLAGS[@]}" -o "$WORK/driver_rust" "$ROOT/migration/checks/utilities_differential.cpp" \
    "$RUST_LIB" "$REALM_LIB" "${FRAMEWORKS[@]}"

# utilities.cpp inlines everything into its five exported functions, so unlike
# base64 there is no file-static table to fingerprint. The guard is instead that each
# driver links exactly one implementation: the Rust archive's probe symbol must be
# present in one and absent from the other. If both were somehow linked, the C++ .o
# would be the only source of the probe-free definitions and this check would catch it.
if ! nm "$WORK/driver_rust" | grep -q "realm_rs_units_ported"; then
  echo "FAIL: driver_rust does not contain the Rust crate probe." >&2
  exit 1
fi
# Both drivers link the full archive, so the decisive check is whether the C++
# translation unit was extracted. a_popcount_bits is utilities.cpp's anonymous-namespace
# lookup table: present means the C++ object won, absent means Rust resolved the symbols
# first and the object was never pulled in.
if nm "$WORK/driver_rust" | grep -q "a_popcount_bitsE"; then
  echo "FAIL: driver_rust contains utilities.cpp's a_popcount_bits table." >&2
  echo "      The C++ object was extracted from the archive, so this comparison" >&2
  echo "      would be C++ against C++ and would pass while proving nothing." >&2
  exit 1
fi
if ! nm "$WORK/driver_cxx" | grep -q "a_popcount_bitsE"; then
  echo "FAIL: driver_cxx does NOT contain utilities.cpp's table -- wrong link." >&2
  exit 1
fi
echo "==> running both"
"$WORK/driver_cxx" > "$WORK/out_cxx.txt"
"$WORK/driver_rust" > "$WORK/out_rust.txt"

lines=$(wc -l < "$WORK/out_cxx.txt" | tr -d ' ')
if [ "$lines" -lt 90 ]; then
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
