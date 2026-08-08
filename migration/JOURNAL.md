# Migration journal

Append-only. One entry per session. This file is why unit N+1 is faster than unit N —
format decisions discovered here do not have to be rediscovered.

Written by `/port-unit` (per unit) and `/reflect` (per session). Never edited to make
anything pass.

---

## 2026-08-08 — harness bootstrap

**Established, with evidence, on this machine:**

- realm-core `v14.14.0-3-gf8752e180` configures and builds against Apple clang 21
  once `-Wno-invalid-specialization` is added. Without it the build fails at
  `upstream/src/external/s2/s1interval.h:189` — s2 specializes `std::is_pod`, which
  clang ≥ 21 rejects outright. Fixed in `harness/CMakeLists.txt`, not upstream.
- **The format is deterministic.** All five traces produced byte-identical files
  across two oracle runs. This is the finding the whole strategy rests on; if it had
  gone the other way, byte-identity would not be a usable acceptance criterion.
- File header decoding, confirmed against real output:
  `[0..16)` two top refs, `[16..20)` `T-DB`, `[20..22)` file format version —
  **one byte per slot, not a u16** — `[22]` reserved, `[23]` flags with bit 0
  selecting the live slot for both the ref and the format byte.
  Observed: `ff…ff 38 03 00 00 00 00 00 00 54 2d 44 42 18 18 00 01`
  → slot 1, top_ref 824, file format 24.
- `realm_compact()` before close removes allocator slack whose size otherwise depends
  on transient allocation order. Without it, file size varies run to run.
- realm's default logger writes compaction timings to stderr. Timings vary per run, so
  the runner sets `RLM_LOG_LEVEL_OFF`.

**Assumptions made without asking:**

- Fixed trace schema: one class `T` with `_id` (int pk), `i` int, `s` string,
  `d` double, `b` bool. Chosen to cover the scalar widths realm packs differently.
  Collections, links, and mixed are not yet exercised — that is a real coverage gap,
  and the first thing to extend once a unit touching them is queued.
- `REALM_ENABLE_SYNC=OFF`, `REALM_APP_SERVICES=OFF`, encryption and geospatial ON.
  Encryption stays on because it changes page layout even when unused.

**Known gap:** no unit has been ported, so `make diff-test` passing proves the harness
works, not that any Rust is correct. The harness has never failed on this machine
except under a deliberate negative control (comparing two different traces' output,
which correctly reported divergence at offset 8).

---

## 2026-08-08 — Phase 0 complete, verified

`make verify` exits 0. Full chain: doctor → oracle → hybrid → determinism-check →
diff-test → format-compat → fault-check.

**Two real bugs found and fixed during setup, both of the silent-green kind:**

1. **Arch mismatch.** This shell runs under Rosetta (`uname -m` = x86_64) while rustc
   is native aarch64. cargo produced an arm64 staticlib, cmake produced an x86_64
   binary, and `ld` **dropped the archive with only a warning and exited 0**. The
   hybrid was silently pure C++ — diff-test would have passed forever while testing
   nothing. Fixed by deriving the Rust triple from `uname -m` in the Makefile.

2. **Archive member never extracted.** Even with matching archs, ld only pulls an
   archive member that resolves an undefined symbol. Nothing referenced the probe, so
   it stayed absent. Fixed with `HARNESS_HYBRID` + an explicit reference in
   trace_runner. `make hybrid` now greps the linked binary for the probe symbol and
   fails the build if it is missing.

The general lesson, worth remembering: **every failure mode found so far during setup
was a false green, not a false red.** Design new checks assuming that is the default.

**`make fault-check` added.** Builds a third stack with one written integer off by one
and requires diff-test to catch it. Result: 5/5 traces caught it. This is the answer to
"how do I know the gate works" and it now runs as part of `make verify`.

**format-compat:** 14 legacy fixtures agree across both stacks, 9 rejected by both
(too old, or need sync history), 0 disagreements.

**queue.md generated:** 103 translation units under `upstream/src/realm/` excluding
object-store/sync/parser/exec/tools/metrics. 14 are depth-0 (no realm-internal
includes) and can be ported without a shim.

**Coverage gap, recorded deliberately:** the trace schema is scalar-only (int, string,
double, bool). No collections, links, Mixed, or Decimal128. Also, `realm_compact()`
runs before every close, so no trace compares an uncompacted file — a free-list bug
that compaction erases would slip through. Both need new traces before any unit
touching them is ported.
