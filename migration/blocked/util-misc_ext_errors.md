# Parked: `upstream/src/realm/util/misc_ext_errors.cpp`

Queue position #1 (depth 0, inbound 0, 29 lines). Parked 2026-08-08.

**Not parked because it is hard. Parked because the gate cannot see it.**

## What I observed

The unit is dead code in this build configuration. Two independent checks agree:

1. **Source level.** Every one of its 33 consumers is under `upstream/src/realm/sync/`,
   and `harness/CMakeLists.txt:20` forces `REALM_ENABLE_SYNC OFF`
   (confirmed in `build/oracle/CMakeCache.txt`: `REALM_ENABLE_SYNC:BOOL=OFF`).

2. **Link level.** The TU compiles into the `Storage` archive, but the linker never
   pulls the object out, because nothing references it:

   ```
   $ nm build/oracle/trace_runner | grep -i MiscExt
   (no output)
   ```

   All 8 symbols the object defines are absent from the linked oracle binary.

There are no divergence offsets to report, and that is precisely the problem.

## Why porting it would be worse than not

`make diff-test` would pass. It would also have passed if I had written an empty file,
or returned the wrong string from every arm of the `switch`. The code is never
executed, never linked, and cannot influence a single byte of any `.realm` file. A
green `make verify` here is not weak evidence — it is *no* evidence, and committing it
would put a unit in `ported_units.txt` whose correctness nothing in this repo checks.

That matters more than usual for this unit, because it is not cheap to port. It is a
`std::error_category` subclass, so replacing it from Rust means synthesizing:

- an Itanium-ABI vtable for `MiscExtErrorCategory`, whose base is libc++'s
  `std::error_category`;
- RTTI (`_ZTI…`, `_ZTS…`) that interoperates with libc++'s own type_info for the
  base, since `std::error_code` comparison and any `dynamic_cast` depend on it;
- a `message(int)` that returns `std::string` **by value**, i.e. reproducing libc++'s
  `std::string` layout including the short-string optimisation;
- the global object `realm::util::misc_ext_error_category` plus its static
  initialiser ordering.

That is a large, fragile, ABI-pinned boundary — the kind that makes `make shim-report`
climb — bought entirely on credit, with no test able to say whether it was paid back.

## What I would need to continue

Any one of these, in rough order of preference:

1. **A reason to believe the unit will ever be reachable.** If sync is eventually
   turned on for the comparison stacks, this unit becomes live and testable, and it
   should be ported *then* — against traces that exercise `network_ssl.cpp`'s
   `end_of_input` / `premature_end_of_input` paths.
2. **A differential check in the style of `migration/checks/base64_differential.cpp`**,
   linking both implementations and comparing `name()` and `message(v)` for every
   enumerator plus out-of-range values. This is the only way to gate it while it stays
   unreachable from traces. It is buildable today, but it validates a unit nothing
   uses, so it is low value until (1).
3. Confirmation that dead-but-present units should be ported for completeness rather
   than skipped. That is a project-scope call, not one I should make silently.

**What I did not do:** enable sync, touch a trace, or touch the comparator. Turning
sync on to make this unit observable would change the flag set for both stacks and
invalidate every prior byte-identity result, and turning it on for one stack is
forbidden outright (CLAUDE.md hard rule 3).

## This is not an isolated case — it is a queue-ordering problem

The same check applied to all 13 depth-0 units. `live` means at least one symbol from
the unit's object file survives into the linked `trace_runner`:

| # | unit | reachable? |
|---|---|---|
| 1 | `util/misc_ext_errors.cpp` | **DEAD** (0/8 symbols linked) |
| 2 | `disable_sync_to_disk.cpp` | live (2/2) |
| 3 | `util/random.cpp` | **DEAD** (0/1) |
| 4 | `util/enum.cpp` | **DEAD** (0/15) |
| 5 | `util/misc_errors.cpp` | **DEAD** (0/2) |
| 6 | `util/cli_args.cpp` | **DEAD** (0/16) |
| 7 | `util/bson/regular_expression.cpp` | **DEAD** (0/14) |
| 8 | `util/basic_system_errors.cpp` | live (2/5) |
| 9 | `util/memory_stream.cpp` | **DEAD** (0/7) |
| 10 | `util/backtrace.cpp` | live (11/20) |
| 11 | `util/base64.cpp` | live (3/6) — ported |
| 12 | `string_data.cpp` | live (5/10) |
| 13 | `error_codes.cpp` | live (7/17) |

Seven of thirteen are dead. `gen_queue.py` orders by dependency depth and size, which
are good proxies for *portability* but say nothing about *observability*, and under a
byte-identity gate observability is what decides whether porting a unit means
anything.

Note that the link-level check catches cases source grep misses:
`util/bson/regular_expression.cpp` has 20 consumers outside `sync/`, all of them in
`bson`, which is itself unreachable with `REALM_APP_SERVICES=OFF`. Transitive
deadness only shows up at the link.

**Consequence for the loop.** `.claude/loop.md` stops after three consecutive parks,
reading it as a wrong queue order. Left alone it would park #1, #3, #4 and halt —
correct behaviour, right diagnosis, but it would burn three iterations to reach a
conclusion already established here. Recommended fix: teach `gen_queue.py` to emit a
reachability column and sort dead units to the back. That is a change to the planning
artifact, not to a gate, so it does not weaken anything.

## Audit 2026-08-08 (reflection #1)

Still blocked. Nothing ported since (`base64`, `disable_sync_to_disk`, `string_data`)
changes reachability — the blocker is that the linker never pulls this object, and no
port can alter that while `REALM_ENABLE_SYNC=OFF`.

One thing did change, and it is worth flagging for whoever picks this up: the
`std::error_category` machinery this unit needs is **also** needed by two units that
*are* live — `util/basic_system_errors.cpp` (#8, 2/5 symbols linked) and
`error_codes.cpp` (#13, 7/17 linked). Once that shim is built for either of them, the
"expensive to port" half of this entry's argument no longer applies.

That does not unpark it. Cost was never the reason. Unreachability is, and it is
unchanged.
