# Parked: `upstream/src/realm/version.cpp`

Queue position #19 (depth 1, inbound 2, 77 lines). Parked 2026-08-09 by reflection #5,
**without an iteration ever opening it** — the park comes out of a re-measurement, not a
port attempt.

## Why this one is worth reading even though the verdict is ordinary

Reflection #4 screened this unit and listed it as a near-queue **candidate**: zero `ZT*`
defined, zero `throw`, zero `try`/`catch`, every exception-handling site owned by a
libc++ weak helper. Clean by steps 2, 5 and 6. The only qualification in that table was
the parenthetical "partial linkage", against a count of **14 defined / 8 linked** — which
reads as mostly reachable and was not treated as disqualifying.

It is disqualifying. All eight survivors are libc++ template instantiations:

```
__ZNSt12length_errorC1B9nqe210106EPKc
__ZNSt3__112basic_stringIcNS_11char_traitsIcEENS_9allocatorIcEEE20__throw_length_errorB9nqe210106Ev
__ZNSt3__115basic_stringbufIcNS_11char_traitsIcEENS_9allocatorIcEEE15__init_buf_ptrsB9nqe210106Ev
__ZNSt3__116__pad_and_outputB9nqe210106IcNS_11char_traitsIcEEEE…
__ZNSt3__118basic_stringstreamIcNS_11char_traitsIcEENS_9allocatorIcEEEC1B9nqe210106Ev
__ZNSt3__118basic_stringstreamIcNS_11char_traitsIcEENS_9allocatorIcEEED1Ev
__ZNSt3__120__throw_length_errorB9nqe210106EPKc
__ZNSt3__124__put_character_sequenceB9nqe210106IcNS_11char_traitsIcEEEE…
```

They arrived with the `<sstream>` use in `Version::get_version()`, they are emitted into
dozens of TUs, and they are in `trace_runner` whether or not this unit is. Not one of
them is anything `version.cpp` is *for*.

Filtering to realm-owned symbols:

```
OBJ=build/oracle/realm-core/src/realm/CMakeFiles/Storage.dir/version.cpp.o
nm -g $OBJ | grep -v ' U ' | awk '{print $NF}' | grep -E '^__ZN[A-Z]*5realm' | sort -u
```

gives six, and **none of the six is in the linked binary**:

```
__ZN5realm7Version11get_versionEv
__ZN5realm7Version11has_featureENS_7FeatureE
__ZN5realm7Version11is_at_leastEiii
__ZN5realm7Version11is_at_leastEiiiNS_10StringDataE
__ZN5realm7Version9get_extraEv
__ZN5realmgeERKNS_10StringDataES2_
```

Confirmed independently: `nm build/oracle/trace_runner | grep Version` returns
`ObjectStore::get_schema_version`, `Transaction::advance_read` and friends — nothing from
`realm::Version`.

## Verdict

**Category 1, unreachable.** `make verify` would pass on an empty Rust file, so a green
gate here records progress nothing can substantiate. Same cause as
[`util-misc_ext_errors.md`](util-misc_ext_errors.md) and the other six sync/tooling-dead
entries; the difference is only that the deadness was hidden behind a plausible-looking
8-of-14.

## Unblocks when

Some trace or corpus file causes `realm::Version` to be linked. Nothing in
`harness/traces/` or `migration/corpus/` reaches version-introspection today, and adding
one is a `harness/` change, which is out of scope by hard rule 2. Treat this as parked
indefinitely rather than pending.

## What it changed elsewhere

The step-1 command in `.claude/rules/evidence-and-linkage.md` now filters to
`^__ZN[A-Z]*5realm`. Re-measured across the near queue, that filter also correctly holds
`util/enum` (5/15 naive → 0/3 realm) and `util/cli_args` (5/16 → 0/7) in this directory —
a naive count would have released two correct parks — and it sharpens
`util/compression`'s park to 4/20, all four being vtable and RTTI scaffolding rather than
any compression function.
