# Bootstrap report

Date: 2026-08-08
Upstream pin: `v14.14.0-3-gf8752e180`

Every claim below cites output observed in this session. Nothing here is asserted on
the grounds that a command "should" pass.

---

## Environment

| Tool | Version |
|---|---|
| cmake | 4.2.2 |
| ninja | 1.13.2 |
| python3 | 3.14.3 |
| cargo | 1.97.0 (c980f4866 2026-06-30) |
| rustc | 1.97.0 (2d8144b78 2026-07-07) |

`make doctor` → exit 0, all rows `ok`, 5 traces registered.

Submodules present and clean at their pinned SHAs:

```
 f8752e180b7f288feadffafdef818068755efe0a upstream (v14.10.0-165-gf8752e180)
 d9ae30f34095107ece9dceb224839f0dc2f9c1c7 upstream/src/external/sha-1 (0.1.0~3)
 0e9aebf34101c6aa89355fd76ac9cd886735dee1 upstream/src/external/sha-2 (heads/master)
 8ac8190e494a381072c89f5e161b92a08d98b37b upstream/test/external/catch (v3.5.3)
```

### Architecture: a live hazard that is currently handled

This machine runs a **Rosetta x86_64 shell over native arm64 hardware**:

```
uname -m: x86_64      arch: i386      rustc host: aarch64-apple-darwin
```

`CMAKE_OSX_ARCHITECTURES` is empty, so the C++ stacks build **x86_64** (confirmed:
`build/oracle/trace_runner: Mach-O 64-bit executable x86_64`). rustc's *default* host
is arm64, and both archives exist on disk:

```
target/release/librealm_core_rs.a                    arm64
target/x86_64-apple-darwin/release/librealm_core_rs.a x86_64
```

The Makefile explicitly cross-builds the x86_64 triple and passes that archive to the
hybrid link (`==> building crates/realm-core-rs for x86_64-apple-darwin`,
`hybrid ready: ... (x86_64-apple-darwin)`). So the mismatch is handled — but it is
handled *by the Makefile*, not by luck. If anyone builds the crate by hand with a bare
`cargo build --release`, the resulting arm64 archive will be silently dropped by `ld`
with only a warning, and the hybrid will link the C++ it was supposed to replace.
**Always go through `make hybrid`.**

---

## What was built

- `make oracle` → exit 0, 0 occurrences of `error:`, `ninja: no work to do` (tree
  already current, flags signature `build/oracle-flags.sig` intact).
- `make hybrid` → exit 0, 0 occurrences of `error:`.
- `make corpus` → exit 0, `seeded 23 legacy realm files into migration/corpus/`,
  0 files matching `decrypt`.

**The hybrid is no longer identical to the oracle.** The skill text anticipates a
"hybrid == oracle by construction" warning; that warning is correctly *absent*,
because `util/base64.cpp` has already landed. Verified structurally rather than
assumed — the C++ translation unit's file-static markers are present in the oracle and
gone from the hybrid:

```
oracle: 00000001003530f0 s __ZN12_GLOBAL__N_114g_base64_charsE
        00000001003530b0 s __ZN12_GLOBAL__N_123g_base64_encoding_charsE
hybrid: (no match)
```

and the Rust definitions carry the C++ mangled names in the hybrid:

```
0000000100016080 T __ZN5realm4util13base64_decodeE...
0000000100016280 T __ZN5realm4util13base64_encodeE...
00000001000163c0 T __ZN5realm4util23base64_decode_to_vectorE...
0000000100016500 T _realm_rs_units_ported
```

This makes the bootstrap a *stronger* check than the one the skill describes: the
diff-test below compares two genuinely different builds, not a build against itself.

---

## Determinism result — the load-bearing step

`make determinism-check` → **exit 0**. Every trace run through the oracle twice and
byte-compared against itself:

```
  det  erase_churn
  det  many_commits
  det  smoke
  det  string_widths
  det  width_boundaries
```

Byte-identity is a usable acceptance criterion on this machine. No offsets to report;
nothing diverged.

### diff-test (oracle vs hybrid)

`make diff-test` → **exit 0**, 5/5 traces. The `rust units ported = 1` line is the
harness confirming it is exercising the Rust path, not a stale binary:

```
harness: rust units ported = 1
  pass erase_churn
  pass many_commits
  pass smoke
  pass string_widths
  pass width_boundaries
```

### The check on the checker

`make verify` runs `fault-check`, which builds a third stack with
`-DHARNESS_FAULT=ON` — documented as *"Perturb one written integer so diff-test MUST
diverge"* — and compares it against the good oracle:

```
  caught erase_churn
  caught many_commits
  caught smoke
  caught string_widths
  caught width_boundaries

gate is live: 5/5 traces detected the injected fault.
```

This matters more than any passing result. A harness that has never failed on this
machine proves nothing when it passes. This one has now failed on demand, on all five
traces, and the failure was detected by the same comparator that gates the port.

---

## Format-compat result

`make format-compat` → **exit 0**. All 23 corpus files accounted for: **14 ok,
9 skip, 0 fail.** Both stacks agree on every file; the skips are files both stacks
reject (agreement is the check, per the harness design).

Opened successfully by both: `downgrade_asymmetric`, `sync-metadata-v4/v5/v6`,
`test_backup-olden-and-golden`, `test_upgrade_database_{11,20,1000_22,1000_23,
1000_10_to_11,4_22,4_23,4_10_to_11}`, `vacuum_no_history_type`.

Rejected by both (skip): `admin_realm_issue_1794`, `client_file_migration_core6`,
`client_schema_version_011/012`, `consistency_full_0/1`, `server_schema_version_020`,
`test_flx_metadata_tables_v1`, `test_upgrade_database_1000_1`.

---

## Queue summary

Regenerated with `python3 migration/gen_queue.py` → `103 units, 14 at depth 0`.
Capped at 60 entries, ordered leaves-first by `(depth, lines, path)`, depth computed
from `#include` edges internal to `upstream/src/realm/` only.

Only **14 of 103** units are depth-0 (shim-free). The port will be shim-heavy early;
that is expected. What is not expected is the `make shim-report` boundary count
continuing to rise across two consecutive reflections.

Regeneration changed the committed file, but **only in the depth ≥ 1 tail** — the
committed copy listed `array_blobs_big.cpp` where the regenerated one lists
`array_blob.cpp`, with knock-on reordering among size-tied units. Entries 1–27 are
unchanged. Since the next unit to port is depth 0, the immediate decision is
unaffected. The committed copy appears to predate a change in the generator; the
regenerated file is now the source of truth.

Head of queue (depth 0), with `util/base64.cpp` at #11 already ported:

| # | unit | inbound | lines |
|---|---|---|---|
| 1 | `util/misc_ext_errors.cpp` | 0 | 29 |
| 2 | `disable_sync_to_disk.cpp` | 3 | 41 |
| 3 | `util/random.cpp` | 0 | 41 |
| 4 | `util/enum.cpp` | 0 | 44 |
| 5 | `util/misc_errors.cpp` | 0 | 68 |

---

## Decisions I made

The skill forbids asking, so these were made unilaterally and are recorded here rather
than escalated.

1. **Proceeded without running `git submodule update --init --recursive`.** The
   command was denied by the sandbox. Rather than treat that as a blocker, I verified
   the requirement it exists to satisfy: all three needed submodules are checked out,
   populated, and clean at their pinned SHAs (output above). The step's purpose is
   met; only its mechanism was unavailable.

2. **Did not force a cold oracle rebuild.** The existing tree reported
   `ninja: no work to do` with its recorded flag signature intact. A cold rebuild
   would have cost 10–20 minutes to reproduce a state already proven current, and
   would not have tested anything the determinism and fault-injection checks don't.

3. **Kept the pre-existing `util/base64.cpp` port in place rather than reverting to a
   pristine hybrid.** The skill assumes bootstrap runs before any unit lands. It
   didn't here — commit `e4843f8` preceded this report. Reverting would have made
   diff-test trivially true and tested less. Verifying against the real hybrid is
   strictly stronger, and the `nm` evidence above establishes the Rust path is live.

4. **Regenerated `migration/queue.md` and accepted the diff** rather than preserving
   the committed ordering, per CLAUDE.md's "do not hand-edit this file — regenerate
   it." Divergence is documented above.

---

## Blockers

**None.**

One standing hazard, currently mitigated and documented above: the Rosetta x86_64
shell / native arm64 rustc split. `make hybrid` handles it by pinning the
`x86_64-apple-darwin` triple. Building the crate outside the Makefile reintroduces a
silent-green failure mode where `ld` discards the wrong-arch archive with a warning
only.

Note on scope, not a blocker: the trace schema exercises one class with `_id`, int,
string, double, and bool — scalar widths only. Collections, links, and Mixed are not
covered. The first unit that touches them needs the traces extended before its green
result means anything.

GO
