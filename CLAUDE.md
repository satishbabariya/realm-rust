# realm-rust — porting realm-core to Rust under a differential oracle

## What this repo is

`upstream/` is a git submodule pinned to realm-core (C++). We are porting it to Rust
**unit by unit**, and the acceptance criterion is not "the tests pass" — it is
**byte-identical `.realm` output** between the C++ oracle and the Rust hybrid for
every trace in `harness/traces/`.

Realm's on-disk format is a memory-mapped B+-tree with element-width-packed arrays.
A port can be behaviourally perfect and still produce an incompatible file (e.g. by
always widening to 64-bit). Byte-identity is the only check that catches that class
of bug, which is why it is the gate.

## The three stacks

| Name | What it is | Built by |
|---|---|---|
| **oracle** | pure C++ realm-core from `upstream/` | `make oracle` → `build/oracle/` |
| **hybrid** | the same C++ tree, with ported units replaced by Rust via `crates/` | `make hybrid` → `build/hybrid/` |
| **rust** | the eventual pure-Rust crate | not buildable standalone yet |

`harness/trace_runner.cpp` is compiled once per stack. Both binaries consume the same
`.trace` files and write `.realm` files. `harness/compare_realm.py` byte-compares them.

## Gate commands — these define "done"

```
make doctor              # toolchain + submodule preflight
make oracle              # build the C++ oracle (slow: 10-20 min cold)
make hybrid              # build the Rust-hybrid stack
make determinism-check   # oracle vs oracle, same trace twice — must be byte-identical
make diff-test           # oracle vs hybrid, every trace — the real gate
make format-compat       # both stacks open every legacy .realm in migration/corpus/
make verify              # doctor + build both + determinism + diff-test + format-compat
```

**`make verify` exiting 0 is the only evidence that a unit is ported.**
"The edit succeeded", "it compiles", and "the tests look right" are not evidence.

## Hard rules

1. **Never edit `upstream/`.** It is the oracle. If it changes, the oracle is no
   longer a reference. A PreToolUse hook blocks writes there.
2. **Never edit `harness/` or `migration/queue.md` to make a check pass.** Weakening
   the definition of done is the fastest path to green and the whole point of the
   harness is to prevent it. If a trace is genuinely wrong, park the unit in
   `migration/blocked/` and explain why — do not edit the trace.
3. **Never change build flags in only one stack.** `make hybrid` refuses to build if
   its flag set differs from what `make oracle` recorded. Flag drift produces byte
   divergence indistinguishable from a porting bug.
4. **One unit per branch, one unit per commit.** A unit is a single `.cpp`/`.hpp` pair
   or a tight cluster from `migration/queue.md`.
5. **Encrypted realms cannot be byte-compared.** Every page write uses a fresh IV.
   Keep encryption compiled in (it changes page layout) but never open a comparison
   realm with a key. `compare_realm.py` detects and refuses these.

## Porting protocol

Use `/port-unit <path>` — it enforces the sequence. The short version:

1. Read the C++ unit *and* its call sites. Note every place element width, alignment,
   or ref encoding is decided.
2. Write the Rust in `crates/realm-core-rs/src/` behind a `#[no_mangle] extern "C"`
   surface matching the C++ symbol exactly.
3. Wire it into the hybrid build by removing the C++ TU from the hybrid target.
4. `make diff-test`. On divergence, read the offset that `compare_realm.py` reports —
   it tells you which array/page disagreed, which is usually a direct pointer to the
   width or alignment decision you got wrong.
5. `make verify`. Then commit.

Record every non-obvious format decision in `migration/JOURNAL.md`. That file is what
makes the next unit faster.

## Stack facts (verified against this checkout, realm-core v14.14.0)

- CMake targets that exist: `Storage`, `ObjectStore`, `QueryParser`, `RealmFFIStatic`, `CoreTests`.
- Build with `REALM_ENABLE_SYNC=OFF`. On Linux/Android, encryption pulls in OpenSSL
  (`CMakeLists.txt:297`) and will try to download a prebuilt tarball; the Makefile
  passes `REALM_USE_SYSTEM_OPENSSL=ON` on non-Darwin. macOS uses SecureTransport and
  needs nothing.
- File header is 24 bytes: `[0..16)` two top refs, `[16..20)` mnemonic `T-DB`,
  `[20..22)` file format version, `[22]` reserved, `[23]` flags (bit 0 selects top ref).
