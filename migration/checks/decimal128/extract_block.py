#!/usr/bin/env python3
"""Slice the vendored Intel code out of upstream/src/realm/decimal128.cpp.

Two blocks, both from the file's anonymous namespace, both *textually* included by the
drivers here rather than linked -- see README.md for why linking is impossible.

  tables_block.inc  typedefs + the seven tables            (decimal128.cpp 62..433)
  conv_block.inc    the above plus realm_binary64_to_bid128 (decimal128.cpp 62..1353)

The slice deliberately omits the opening `namespace {` and its closing `}` so the driver
can place the block in a namespace of its own; each output is asserted brace-balanced.

Run from the repo root:  python3 migration/checks/decimal128/extract_block.py <outdir>
"""
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SRC = ROOT / "upstream" / "src" / "realm" / "decimal128.cpp"


def main() -> int:
    outdir = Path(sys.argv[1]) if len(sys.argv) > 1 else Path.cwd()
    lines = SRC.read_text().splitlines()

    # Anchor on content, not line numbers: upstream is a pinned submodule today, but a
    # silent re-slice at the wrong offset would produce a driver that compiles and
    # compares the wrong thing.
    if lines[59].strip() != "namespace {":
        print(f"decimal128.cpp:60 is {lines[59]!r}, expected 'namespace {{'", file=sys.stderr)
        return 2
    try:
        tables_end = next(i for i, l in enumerate(lines)
                          if l.startswith("void realm_binary64_to_bid128"))
    except StopIteration:
        print("could not find realm_binary64_to_bid128", file=sys.stderr)
        return 2
    # The FIRST '} // namespace' closes the earlier anonymous namespace at line 46
    # (to_decimal128 / to_BID_UINT128), not this one -- search past the function.
    try:
        conv_end = next(i for i, l in enumerate(lines)
                        if i > tables_end and l.startswith("} // namespace"))
    except StopIteration:
        print("could not find the closing '} // namespace'", file=sys.stderr)
        return 2

    for name, end in (("tables_block.inc", tables_end), ("conv_block.inc", conv_end)):
        block = "\n".join(lines[60:end])
        bal = block.count("{") - block.count("}")
        if bal != 0:
            print(f"{name}: brace balance {bal}, refusing to write", file=sys.stderr)
            return 2
        (outdir / name).write_text(block)
        print(f"  {name}: {len(block.splitlines())} lines, balanced")
    return 0


if __name__ == "__main__":
    sys.exit(main())
