#!/usr/bin/env python3
"""Byte-compare two .realm files and localise the first divergence.

The offset is the useful output. Realm lays out arrays with a packed element
width chosen per-array, so a divergence at the first byte of an array header is
almost always a width decision, and a divergence in the payload with matching
headers is almost always an encoding or endianness decision. Reporting the page
and the surrounding bytes turns "the port is wrong" into "this array is wrong".

Exit codes: 0 identical, 1 divergent, 2 unusable input.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

HEADER_SIZE = 24
MNEMONIC_OFFSET = 16
MNEMONIC = b"T-DB"
FORMAT_VERSION_OFFSET = 20
FLAGS_OFFSET = 23

# Realm maps in page-sized chunks; 4K and 16K both occur in the wild. Reporting
# both indices costs nothing and saves a mental division at 2am.
PAGE_SIZES = (4096, 16384)

CONTEXT = 32


def read(path: Path) -> bytes:
    try:
        return path.read_bytes()
    except OSError as exc:
        print(f"compare_realm: cannot read {path}: {exc}", file=sys.stderr)
        raise SystemExit(2)


def looks_encrypted(data: bytes, path: Path) -> bool:
    """An encrypted realm has no plaintext T-DB mnemonic in its first block.

    Encrypted files can never be byte-compared: each page write uses a fresh IV,
    so two files differ even when produced by identical code. Saying so is much
    more useful than reporting a phantom divergence at offset 0.
    """
    if len(data) < HEADER_SIZE:
        print(f"compare_realm: {path} is {len(data)} bytes, too short to be a realm", file=sys.stderr)
        raise SystemExit(2)
    return data[MNEMONIC_OFFSET:MNEMONIC_OFFSET + 4] != MNEMONIC


def describe_header(data: bytes) -> str:
    # m_file_format is uint8_t[2] — one version per top-ref slot, not a single
    # little-endian u16. Bit 0 of flags selects which slot is live, for both the
    # ref and the format byte.
    flags = data[FLAGS_OFFSET]
    slot = flags & 1
    fmt = data[FORMAT_VERSION_OFFSET + slot]
    top_ref = int.from_bytes(data[8:16] if slot else data[0:8], "little")
    return f"file_format={fmt} flags=0x{flags:02x} slot={slot} top_ref={top_ref}"


def hexdump(data: bytes, centre: int, label: str) -> str:
    start = max(0, centre - CONTEXT // 2)
    end = min(len(data), centre + CONTEXT // 2)
    chunk = data[start:end]
    rel = centre - start
    hexs = " ".join(f"{b:02x}" for b in chunk)
    caret = " " * (rel * 3) + "^^"
    return f"  {label} @{start:#x}: {hexs}\n  {' ' * len(label)}  {caret}"


def region(offset: int) -> str:
    if offset < HEADER_SIZE:
        names = {
            (0, 8): "header: top_ref[0]",
            (8, 16): "header: top_ref[1]",
            (16, 20): "header: mnemonic",
            (20, 21): "header: file format version (slot 0)",
            (21, 22): "header: file format version (slot 1)",
            (22, 23): "header: reserved",
            (23, 24): "header: flags (bit0 selects top ref)",
        }
        for (lo, hi), name in names.items():
            if lo <= offset < hi:
                return name
    pages = ", ".join(f"{ps // 1024}K page {offset // ps} +{offset % ps:#x}" for ps in PAGE_SIZES)
    return f"body ({pages})"


def compare(a_path: Path, b_path: Path, a_label: str, b_label: str) -> int:
    a, b = read(a_path), read(b_path)

    for data, path in ((a, a_path), (b, b_path)):
        if looks_encrypted(data, path):
            print(
                f"compare_realm: {path} has no plaintext 'T-DB' mnemonic — it is encrypted "
                f"or not a realm file.\n"
                f"  Encrypted realms cannot be byte-compared: every page write uses a fresh IV, "
                f"so two files differ even when the code is identical.\n"
                f"  Re-run the trace without an encryption key.",
                file=sys.stderr,
            )
            return 2

    if a == b:
        print(f"IDENTICAL  {len(a)} bytes  ({describe_header(a)})")
        return 0

    print(f"DIVERGENT")
    print(f"  {a_label}: {a_path}  {len(a)} bytes  ({describe_header(a)})")
    print(f"  {b_label}: {b_path}  {len(b)} bytes  ({describe_header(b)})")

    limit = min(len(a), len(b))
    first = next((i for i in range(limit) if a[i] != b[i]), None)

    if first is None:
        print(f"\n  Common prefix of {limit} bytes is identical; the files differ only in length.")
        print(f"  Size delta: {len(b) - len(a):+d} bytes.")
        print("  A pure length difference usually means an allocator or compaction decision,")
        print("  not a value encoding one.")
        return 1

    print(f"\n  First divergence at offset {first} ({first:#x}) — {region(first)}")
    print(f"    {a_label} = 0x{a[first]:02x}   {b_label} = 0x{b[first]:02x}")
    print(hexdump(a, first, a_label))
    print(hexdump(b, first, b_label))

    differing = sum(1 for i in range(limit) if a[i] != b[i])
    print(f"\n  {differing} of {limit} common bytes differ ({100.0 * differing / limit:.2f}%).")
    if differing > limit * 0.5:
        print("  More than half the bytes differ — suspect a wholesale layout or flag")
        print("  difference between the two stacks, not a single bad value.")
    return 1


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("a", type=Path)
    p.add_argument("b", type=Path)
    p.add_argument("--label-a", default="A")
    p.add_argument("--label-b", default="B")
    args = p.parse_args()
    return compare(args.a, args.b, args.label_a, args.label_b)


if __name__ == "__main__":
    sys.exit(main())
