---
description: What counts as evidence that a unit is ported, and how to prove the Rust actually replaced the C++. Derived from repeated findings across the first four porting sessions.
paths:
  - "crates/**/*.rs"
  - "migration/**"
---

# Evidence and linkage

`format-fidelity.md` covers how to get the bytes right. This file covers the prior
question: whether any check in this repo can tell that you did.

Every failure mode found in this repo so far has been a **false green**, never a false
red. Design your checks assuming that is the default.

## Classify the unit's observability before you read its C++

Do this first, before opening the source. It costs one command and it has reordered the
queue twice.

```
OBJ=build/oracle/realm-core/src/realm/CMakeFiles/Storage.dir/<unit>.cpp.o
nm -g $OBJ | grep -v ' U ' | awk '{print $NF}' | grep '^__Z' | sort -u  # defined
nm build/oracle/trace_runner | awk '{print $NF}' | sort -u              # linked
```

Intersect with `comm -12`. (The object path had `realm-core/src/realm/` missing until
2026-08-09; realm-core is an `add_subdirectory`, so the objects are three levels below
`build/oracle/`.)

Intersect them. Four categories, and they need different evidence:

| Category | Test | Example | What `make diff-test` proves |
|---|---|---|---|
| **Unreachable** | 0 symbols survive into the linked binary | `util/misc_ext_errors`, `util/random`, 5 more | nothing — it passes for an empty file |
| **Reachable, byte-invisible** | symbols linked, but output never reaches a `.realm` | `disable_sync_to_disk`, `util/base64` | the link is intact, nothing more |
| **Byte-visible, untraced** | could write file bytes, but no trace exercises that path | `string_data` (no trace builds a string index) | only that nothing else regressed |
| **Byte-visible and traced** | a wrong byte fails a trace | `array_unsigned` — the first, 2026-08-09 | this is the real gate, and it is **shallower than it looks**; see below |

Rules that follow:

- **Unreachable → park it.** Do not port it. A green `make verify` on a unit the linker
  never pulls is not weak evidence, it is no evidence, and committing it records
  progress nothing can substantiate.
- **Source grep is not the test; the link is.** `util/bson/regular_expression.cpp` has
  20 consumers outside `sync/`, all in `bson`, which is itself dead with
  `REALM_APP_SERVICES=OFF`. Grep called it live. The link did not. Transitive deadness
  is only visible at the link.
- **Byte-visible-but-untraced → the differential in `migration/checks/` is the
  evidence**, and `make verify` is the regression check. Write the differential
  *before* the Rust; that is where the bug will be. Two templates exist:
  `run_base64_differential.sh` and `run_string_data_differential.sh`. Both caught, or
  were built to catch, things nothing else could see.
- **Reachable-but-byte-invisible and small → unit tests are enough.** The
  "always write a differential" advice does not scale down. A 208-line harness
  comparing a bool getter against a bool getter is ceremony, not evidence
  (`disable_sync_to_disk`). Say in the journal that you skipped it and why.

## Linkage is not coverage — measure which traces actually call the unit

The table above classifies a unit by whether its symbols survive to the link. That is a
**necessary** condition for the gate to see it and not a sufficient one, and the gap was
measured for the first time on `array_unsigned`:

| Injected bug in the landed Rust | `make diff-test` | the unit's differential |
|---|---|---|
| `bit_width`: `value < 0x10000` → `<= 0x10000` (differs only at 65536) | **passes, exit 0** | fails, 22 lines |
| `bit_width`: small values return 16 instead of 8 (differs everywhere) | fails (`erase_churn`, `many_commits`) | fails |

So the gate does judge a byte-visible unit — the broad bug is caught — but the traces
only ever drive `ArrayUnsigned` through its 8-bit path. A width bug that first bites at
the 16-, 32- or 64-bit boundary is invisible to `make verify`. That is not a contrived
bug class; it is the one `format-fidelity.md` opens with.

**After a port lands and before writing down what the gate proved, instrument it.** One
`AtomicUsize` per exported symbol, printed on the 0→1 transition, then `make hybrid` and
run each trace. Five minutes, and it converts "the linker kept these symbols" into
"these traces call these functions". For `array_unsigned` it reported 6 of 10 functions
and 2 of 5 traces, with `set`, `truncate` and `upper_bound` reached by nothing — which
is what the differential has to cover.

Corollary, because it nearly went the other way: **do not infer coverage from a trace's
name.** `width_boundaries.trace` does not reach `ArrayUnsigned` at all. It exercises
width packing in `Array`.

## Prove the Rust actually replaced the C++

Three separate silent-green failures have had the same shape: **the absence of the Rust
was indistinguishable from its presence.**

1. Rosetta x86_64 shell vs native arm64 rustc — `ld` dropped the staticlib with a
   warning and exited 0. The hybrid was pure C++ and diff-test was green.
2. `ld` extracts an archive member only to resolve an undefined symbol. Nothing
   referenced the crate, so it was never extracted.
3. With multiple codegen units, only the CGU containing the probe symbol gets
   extracted. A ported unit in another CGU is silently supplied by `librealm.a`
   instead. `codegen-units = 1` in `[profile.release]` exists for this and must stay —
   it is what makes "the probe is linked" and "every ported unit is linked" the same
   statement.

So after every port, check both directions on the hybrid binary:

- the C++ TU's private symbols (a file-static table, an anonymous-namespace global) are
  **absent**, and
- the Rust symbols are **present** at the expected offsets.

If the unit has no file-static to fingerprint (`string_data.cpp` inlines everything
into its exports), assert instead that the crate probe is present in the Rust-linked
driver and absent from the C++ one.

The hybrid excludes C++ purely by **link order** — `librealm_core_rs.a` precedes
`librealm.a` on the link line. There is no build-system change to make, and none is
possible: `harness/**` and the `Makefile` are permission-denied by design. The comment
in `harness/CMakeLists.txt` about a `build.rs` is wrong; there is no `build.rs`.

## Choosing the shape of a differential

Two shapes are in the tree and they are not interchangeable. Decide with `nm -u` on the
unit's object **before** writing the harness, not after the link fails.

| `nm -u <unit>.o` shows | Shape | Example |
|---|---|---|
| a short, self-contained undefined list (system libs only) | **single-object linking** — each driver links the one `.o` or the one Rust module | `run_interprocess_mutex_differential.sh` (4 undefined, all libdispatch) |
| anything pulling in `util::Mutex`, `terminate`, or `Backtrace` | **whole-archive + link order** — both drivers link all of `librealm.a`, the Rust driver puts its staticlib first | `run_utilities_differential.sh` |

Single-object linking fails on units with dependency chains: `utilities.cpp`'s
file-static `util::Mutex` drags in `Mutex::*_failed` from `thread.cpp`, which drags in
`terminate.cpp` and `backtrace.cpp`. Linking that chain object-by-object is a losing
game.

Prefer the archive shape when in doubt. It does not care how many TUs a unit depends
on, and it exercises the exact mechanism `make hybrid` uses, so a link-order bug shows
up in the differential rather than only in the gate.

The guard changes with the shape. Single-object: each driver contains exactly one
implementation, so the crate probe suffices. Archive: assert that the C++ object was
*not extracted*, fingerprinting on an anonymous-namespace symbol from the unit
(`a_popcount_bits` for `utilities.cpp`). Find that fingerprint symbol first — if the
unit has none because it inlines everything into its exports (`string_data.cpp`), fall
back to the crate probe.

## Get the ABI off the built object, not out of your head

Ten minutes with `objdump -d` has twice settled something that would otherwise have
been a guess (`std::optional<size_t>` returned in `rax:dl`; `optional<vector<char>>`
returned via `sret` in `rdi`). The layout table lives in `JOURNAL.md` under the base64
entry.

Two traps confirmed on this machine:

- **Buffers handed back to C++ must come from `operator new`** (`_Znwm` / unsized
  `_ZdlPv`, declared `extern`). C++ destroys them; a Rust-allocator block would be
  freed by the wrong allocator. And an empty `std::vector<char>` is three null
  pointers, not a zero-size allocation.
- **On Darwin an `asm` label is used verbatim**, so binding a private mangled C++
  symbol needs the leading underscore written by hand: `__ZN5realm…`, two underscores.
  This is how `matchlike`/`matchlike_ins` were reached without depending on an inline
  wrapper surviving optimisation.

## Assertions and overflow

`REALM_ASSERT`, `REALM_ASSERT_EX`, `REALM_ASSERT_DEBUG` and `REALM_ASSERT_3/7/11` are
no-ops in this build. The confirmation is the **build configuration**, not the objects:
`build/oracle/CMakeCache.txt` has `REALM_ENABLE_ASSERTIONS:BOOL=OFF`,
`CMAKE_CXX_FLAGS_RELEASE` is `-O3 -DNDEBUG`, and nothing defines `REALM_DEBUG`; by
`util/assert.hpp:25-44` each of those macros then expands to
`static_cast<void>(sizeof bool(...))`, evaluated for type and never executed.

> **Corrected 2026-08-09 (reflection #3).** This paragraph previously said the no-op
> status was "confirmed by the absence of an undefined `realm::util::terminate` in the
> objects". That test is wrong and would misclassify most of the library:
> `REALM_ASSERT_RELEASE` (`assert.hpp:31`) and `REALM_UNREACHABLE()` (`assert.hpp:99`)
> sit under no `#if` and always call `realm::util::terminate`, so **42 of the 67
> `Storage` objects have `terminate` undefined** while all of their `REALM_ASSERT*`
> uses are dead. The conclusion held for the units ported so far by luck of which ones
> they were. Read the flags, then grep the unit for `REALM_ASSERT_RELEASE` and
> `REALM_UNREACHABLE` — those two must be reproduced in Rust as a call to
> `realm::util::terminate(msg, file, line)`, not as a panic and not as
> `unreachable_unchecked`.

Because the gated asserts are dead, **release arithmetic can overflow exactly as the C++
does**, and the workspace sets `overflow-checks = true`,
which would panic where the C++ wraps. Use `wrapping_*` wherever you are mirroring
arithmetic that upstream lets overflow, and say so in a comment.
