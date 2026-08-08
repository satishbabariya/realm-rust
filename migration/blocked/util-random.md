# Parked: `upstream/src/realm/util/random.cpp`

Queue position #3 (depth 0, inbound 0, 41 lines). Parked 2026-08-08.

Same cause as [`util-misc_ext_errors.md`](util-misc_ext_errors.md): unreachable in this
build configuration, so the byte-identity gate cannot distinguish a correct port from
an empty one.

## What I observed

The TU defines exactly one external symbol:

```
$ nm -g build/oracle/.../Storage.dir/util/random.cpp.o | grep -v " U "
0000000000000000 T __ZN5realm5_impl22get_extra_seed_entropyERjS1_S1_
                    → realm::_impl::get_extra_seed_entropy(unsigned&, unsigned&, unsigned&)
```

and that symbol is absent from the linked oracle binary. Consumers:

| Consumer | Reachable here? |
|---|---|
| `sync/network/default_socket.cpp:486` | no — `REALM_ENABLE_SYNC=OFF` |
| `sync/noinst/server/server.cpp:3734` | no — sync |
| `sync/noinst/client_impl_base.cpp:157` | no — sync |
| `test/test_util_compression.cpp:45` | no — not linked into `trace_runner` |
| `test/fuzz/fuzz_transform.cpp:46` | no — comment only |

## Why porting it would prove nothing

`make diff-test` would pass for any implementation. Nothing calls it.

There is a second, independent reason this unit can never be validated by byte
comparison, and it is worth recording because it generalises: **the function is
deliberately nondeterministic.** It returns

- `high_resolution_clock::now().time_since_epoch().count()`
- `getpid()`
- `++g_counter`

If its output ever influenced file content, `make determinism-check` would fail by
construction — the same run twice would differ. So byte-identity is not merely blind to
this unit, it is *structurally incapable* of gating it. Any future check on it has to
assert on the shape of the entropy (three distinct values written, counter strictly
increasing across calls), never on bytes.

## What I would need to continue

1. Sync enabled for the comparison stacks, which would make the consumers real. Not a
   change I can make: it alters the flag set for both stacks and invalidates every
   prior byte-identity result (CLAUDE.md hard rule 3 forbids changing it for one).
2. Failing that, a `migration/checks/` differential asserting the entropy contract
   rather than the values — cheap to write, but it gates a function nothing calls.

Did not touch a trace, the comparator, the Makefile, or `upstream/`.

## Loop status

This is the **second consecutive park**. Queue #4 (`util/enum.cpp`) is dead by the same
measurement, so the next tick would be the third and `.claude/loop.md` would stop the
loop on its three-consecutive-parks rule.

That rule is doing its job — the queue order *is* wrong — but the diagnosis is already
in hand and does not need a third park to establish it: seven of the thirteen depth-0
units are unreachable, and `gen_queue.py` cannot see it because it sorts by dependency
depth and size. The fix is a reachability column in the generator, sorting dead units to
the back. That is a change to the planning artifact, not to any gate.
