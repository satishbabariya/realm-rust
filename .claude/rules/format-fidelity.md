---
description: On-disk format rules for porting realm-core to Rust. These describe how the file is laid out and what breaks it.
paths:
  - "crates/**/*.rs"
  - "harness/**"
---

# Format fidelity

The tests do not test the thing this project is about. Behaviour is easy; layout is the
deliverable. These are the rules that produce layout bugs when broken.

## Element width is not an optimisation

Realm packs every integer array at the narrowest width that fits its current range:
0, 1, 2, 4, 8, 16, 32, or 64 bits. The width is stored in the array header and the
payload is bit-packed accordingly.

A port that stores everything as `i64` is functionally perfect and produces a file no
other realm binding can read. When porting anything that writes an array:

- Find where the C++ decides the width. Mirror the calculation, do not re-derive it.
- Widths are **signed**: `-1` and `1` do not cost the same number of bits. A port that
  computes width from the unsigned magnitude diverges only on negative values, which
  is exactly the kind of bug that survives a casual test suite.
- Widening is monotonic within an array's lifetime — arrays widen on insert and do not
  narrow on erase. If your port narrows, it is smaller *and* wrong.

## Refs are tagged offsets

A ref is a byte offset into the file. The low bit tags an inline value rather than a
pointer. Never treat a ref as an opaque integer you can round-trip through anything
that might sign-extend or normalise it.

## Mirror the arithmetic, including where it looks wrong

If the C++ relies on implementation-defined behaviour — shift semantics, integer
promotion, signed overflow — reproduce the observed behaviour and comment it as a
deliberate mirror. Rust's defined semantics differ from C++'s undefined ones in ways
that change bytes. "I cleaned this up" is a bug report.

## Everything crossing the boundary is `#[repr(C)]`

Rust's default layout is unspecified and free to reorder fields. Any struct that
crosses the FFI boundary or reaches the file needs `#[repr(C)]`, and any struct whose
size matters needs a `const_assert` on `size_of`.

## The free list is part of the format

Allocation order and free-list ordering are written to the file and read by the next
writer. Two allocators that satisfy the same requests in a different order produce
different, mutually-readable-but-not-identical files. `erase_churn.trace` exists
specifically to catch this; if it is the only trace failing, look at allocation order
before you look at values.

## Encrypted realms are never byte-compared

Every page write uses a fresh IV, so two encrypted files differ even from identical
code. Keep encryption compiled in — it changes page layout even when unused — but
never open a comparison realm with a key. `compare_realm.py` detects this and refuses.

## Reading a divergence offset

| Offset region | First thing to check |
|---|---|
| `[0..16)` | top ref for the live slot (bit 0 of flags at `[23]` selects it) |
| `[16..20)` | mnemonic `T-DB` — if this differs, the file is encrypted or not a realm |
| `[20..22)` | file format version, one byte per slot, selected by the same flag bit |
| body, single byte, low bits | element width |
| body, payload differs, header matches | value encoding or endianness |
| length only | allocator or compaction |
| >50% of bytes | flag drift between stacks — rebuild both, this is not a port bug |
