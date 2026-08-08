# Parked: `upstream/src/realm/util/cli_args.cpp`

Queue position #6 (depth 0). Parked 2026-08-08.

Unreachable in this build configuration, so the byte-identity gate cannot distinguish a
correct port from an empty one. Same cause and same measurement as
[`util-misc_ext_errors.md`](util-misc_ext_errors.md), which carries the full analysis
and the reachability table for all thirteen depth-0 units.

## Evidence

Intersecting the symbols the unit's object file defines with those present in the
linked oracle binary:

```
util/cli_args.cpp.o: 0/16 symbols linked into build/oracle/trace_runner
```

Command-line argument parsing, used by realm's standalone tools rather than by the
library. No includer under `upstream/src/`.

## Why parked rather than ported

`make diff-test` would pass for any implementation, including an empty one, because
nothing calls into this translation unit. A green `make verify` here would record
progress that no check in this repo can substantiate.

## What I would need to continue

Reachability. For the sync-only units that means `REALM_ENABLE_SYNC=ON` for both
comparison stacks — which changes the flag set and invalidates every prior
byte-identity result, so it is a project-level decision, not one to take mid-port.
Failing that, a `migration/checks/` differential in the style of
`run_string_data_differential.sh`, which gates a unit the traces cannot reach; cheap to
write, but low value while nothing calls the code.

Did not touch a trace, the comparator, the Makefile, or `upstream/`.
