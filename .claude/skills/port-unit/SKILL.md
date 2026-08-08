---
name: port-unit
description: Port one realm-core translation unit from C++ to Rust and prove byte-identity against the oracle. Takes a path under upstream/src/realm/, or picks the next unblocked unit from migration/queue.md.
disallowed-tools: AskUserQuestion
---

# Port one unit

Target: **$1** (if empty, take the first unit in `migration/queue.md` that is not
already ported and has no file in `migration/blocked/`).

Queue: !`head -30 migration/queue.md 2>/dev/null || echo "no queue — run /bootstrap"`
Blocked: !`ls migration/blocked/ 2>/dev/null | tr '\n' ' '`
Branch: !`git branch --show-current`

You may not ask the user anything. Resolve ambiguity by matching the conventions of
an already-ported unit; if none exists, choose and record the choice in
`migration/JOURNAL.md` under "Assumptions".

## Sequence

### 1. Read before writing

Read the C++ unit **and its call sites**. `grep -rn` the header name across
`upstream/src/realm/`. You are looking specifically for every point where one of
these is decided, because these are what byte-identity actually tests:

- **element width** — realm packs arrays at the narrowest width that fits (0/1/2/4/8/16/32/64 bits)
- **alignment and padding** of on-disk structures
- **ref encoding** — refs are byte offsets with the low bit tagging inline values
- **endianness** of anything written to the file
- **allocation and free-list ordering** — visible on disk, invisible to tests

List these explicitly before writing any Rust. If you cannot find where width is
decided for a unit that writes arrays, you do not understand the unit well enough
to port it yet — say so and park it.

### 2. Write the Rust

In `crates/realm-core-rs/src/`. The surface is `#[no_mangle] pub extern "C"` with
symbol names matching the C++ mangled symbols the hybrid link needs to satisfy.

Idiomatic Rust is welcome *behind* the boundary. It is not welcome in anything that
decides layout: mirror the C++ arithmetic exactly, including any place it relies on
implementation-defined behaviour, and comment the mirror where it looks wrong. A
"cleaner" width calculation that produces a different width is a compatibility break,
not an improvement.

Use `#[repr(C)]` on every struct that crosses the boundary or reaches the file.

### 3. Wire it into the hybrid

Remove the C++ TU from the hybrid target so the Rust definition is the one that
links. Never remove it from the oracle.

### 4. Prove it

```
make diff-test
```

This is the only step that establishes anything. On divergence, read the offset
`compare_realm.py` reports:

| What you see | What it usually means |
|---|---|
| divergence in the header `[0..24)` | top-ref or format-version handling |
| single byte, low bits differ | element width chosen differently |
| whole array payload differs, header matches | value encoding or endianness |
| only file length differs | allocator or compaction decision |
| >50% of bytes differ | flag drift between stacks, not a porting bug — run `make clean && make oracle hybrid` |

Fix the Rust. Never the trace, never the comparator, never upstream.

### 5. Full gate, then commit

```
make verify
```

Must exit 0. Then one commit for the one unit, message naming the unit and any format
decision you had to reverse-engineer.

### 6. Journal

Append to `migration/JOURNAL.md`: the unit, the format decisions you found, anything
that surprised you, and how long it took. This file is the reason unit N+1 is faster
than unit N.

## When to stop and park

Write `migration/blocked/<unit>.md` and move on if any of these is true. Parking is a
successful outcome; grinding is not.

- The unit's format behaviour depends on something not yet ported.
- `make diff-test` fails and you have made no progress on the offset across three
  distinct hypotheses.
- You would need to change a trace, the comparator, the Makefile, or `upstream/` to
  pass. This one is absolute — parking is the *only* correct response.

The file must state: what you tried, what you observed (with offsets), what you think
is happening, and what you would need in order to continue. "Blocked" with no
diagnosis is worse than not attempting the unit.

## Not evidence

A compile that succeeds. An edit that applies. Tests that look right. Reasoning about
why it must be correct. Only `make verify` output showing exit 0 is evidence, and you
must have seen it in this session.
