#!/usr/bin/env python3
"""Write a corpus of doubles (raw little-endian u64 bit patterns) to stdout.

`Decimal128(double, RoundTo)` is a pure function of one double, which is a rare luxury:
the input domain can be attacked directly instead of sampled through an API. 2**64 is
still out of reach, so the corpus is built from the places a decimal-conversion bug
actually lives, plus a large deterministic random tail.

Deterministic: seeded, so a failure is reproducible and the harness can be re-run.
"""
import random
import struct
import sys

def main() -> int:
    n_random = int(sys.argv[1]) if len(sys.argv) > 1 else 2_000_000
    out = sys.stdout.buffer
    seen = set()

    def put(bits: int) -> None:
        bits &= (1 << 64) - 1
        if bits not in seen:
            seen.add(bits)
            out.write(struct.pack("<Q", bits))

    def putf(x: float) -> None:
        put(struct.unpack("<Q", struct.pack("<d", x))[0])

    # Specials and both zeros/infinities/NaNs, quiet and signalling.
    for b in (0x0000000000000000, 0x8000000000000000,      # +0, -0
              0x7FF0000000000000, 0xFFF0000000000000,      # +inf, -inf
              0x7FF8000000000000, 0xFFF8000000000000,      # quiet NaN
              0x7FF0000000000001, 0x7FF4000000000000):     # signalling NaN
        put(b)

    # Every exponent, at the significand extremes and just off them. This sweeps the
    # normal/denormal boundary and the whole e range the function branches on.
    for e in range(0, 2048):
        for m in (0, 1, 2, 3, (1 << 52) - 3, (1 << 52) - 2, (1 << 52) - 1, 1 << 51, 0xAAAAAAAAAAAAA):
            for s in (0, 1):
                put((s << 63) | (e << 52) | (m & ((1 << 52) - 1)))

    # Smallest/largest denormals and normals, and their neighbours.
    for b in (1, 2, 3, (1 << 52) - 1, 1 << 52, (1 << 52) + 1, 0x7FEFFFFFFFFFFFFF):
        put(b)
        put(b | (1 << 63))

    # Exact powers of ten and two, where the "integer fits the coefficient" fast paths
    # at decimal128.cpp:557 and :576 switch over, plus their immediate neighbours.
    for k in range(-323, 309):
        try:
            putf(float(f"1e{k}"))
        except (OverflowError, ValueError):
            pass
    for k in range(-1074, 1024):
        try:
            putf(2.0 ** k)
        except OverflowError:
            pass
    # a == 48 is the cutoff in the second fast path (5**49 > 10**34); walk across it.
    for a in range(40, 56):
        putf(2.0 ** -a)
        putf(3.0 * 2.0 ** -a)

    # Values whose shortest decimal form is exactly 15 or 17 digits -- the boundary the
    # RoundTo::Digits15 quantize step is built around.
    for i in range(1, 4000):
        putf(i / 7.0)
        putf(i * 1e15 + 0.5)

    rng = random.Random(0x0DEC1AA1)
    for _ in range(n_random):
        put(rng.getrandbits(64))

    return 0

if __name__ == "__main__":
    sys.exit(main())
