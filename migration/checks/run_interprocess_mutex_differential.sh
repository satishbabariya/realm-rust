#!/usr/bin/env bash
# Differential check for util/interprocess_mutex.cpp (SemaphoreMutex).
#
# The unit never touches a .realm, so diff-test cannot see it. This pins the observable
# contract of the binary semaphore instead, and -- because each driver links exactly one
# implementation -- proves the Rust definitions are the ones being exercised.
#
# Run after `make oracle hybrid`. Exits non-zero on any disagreement.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ORACLE_DIR="$ROOT/build/oracle"
HYBRID_DIR="$ROOT/build/hybrid"
WORK="$ROOT/build/out/interprocess-mutex-diff"

CXX_OBJ="$ORACLE_DIR/realm-core/src/realm/CMakeFiles/Storage.dir/util/interprocess_mutex.cpp.o"
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
c++ "${CXXFLAGS[@]}" -o "$WORK/driver_cxx" "$ROOT/migration/checks/interprocess_mutex_differential.cpp" "$CXX_OBJ"

echo "==> building driver against the Rust staticlib"
c++ "${CXXFLAGS[@]}" -o "$WORK/driver_rust" "$ROOT/migration/checks/interprocess_mutex_differential.cpp" "$RUST_LIB" \
    -framework Foundation -framework Security -lz -lcompression

# util/interprocess_mutex.cpp inlines everything into its five exported functions, so unlike
# base64 there is no file-static table to fingerprint. The guard is instead that each
# driver links exactly one implementation: the Rust archive's probe symbol must be
# present in one and absent from the other. If both were somehow linked, the C++ .o
# would be the only source of the probe-free definitions and this check would catch it.
if ! nm "$WORK/driver_rust" | grep -q "realm_rs_units_ported"; then
  echo "FAIL: driver_rust does not contain the Rust crate probe." >&2
  echo "      The Rust staticlib was not linked; this comparison would be C++" >&2
  echo "      against C++ and would pass while proving nothing." >&2
  exit 1
fi
if nm "$WORK/driver_cxx" | grep -q "realm_rs_units_ported"; then
  echo "FAIL: driver_cxx contains the Rust crate probe — wrong archive linked." >&2
  exit 1
fi
for sym in _ZN5realm4util14SemaphoreMutex4lockEv _ZN5realm4util14SemaphoreMutexC1Ev _ZN5realm4util14SemaphoreMutexD1Ev; do
  if ! nm "$WORK/driver_rust" | grep -q "T _${sym}\$"; then
    echo "FAIL: ${sym} is not defined in driver_rust." >&2
    exit 1
  fi
done

echo "==> running both"
"$WORK/driver_cxx" > "$WORK/out_cxx.txt"
"$WORK/driver_rust" > "$WORK/out_rust.txt"

lines=$(wc -l < "$WORK/out_cxx.txt" | tr -d ' ')
if [ "$lines" -lt 25 ]; then
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
