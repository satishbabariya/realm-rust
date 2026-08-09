# Parked: `upstream/src/realm/util/resource_limits.cpp`

122 lines, four exported symbols, **none of them linked**. Parked 2026-08-09 at step 1.
The simplest park in the queue — no ABI question, no shim, no judgement call.

```
nm -g util/resource_limits.cpp.o | grep -v ' U '
  T realm::util::system_has_rlimit(realm::util::Resource)
  T realm::util::get_hard_rlimit(realm::util::Resource)
  T realm::util::get_soft_rlimit(realm::util::Resource)
  T realm::util::set_soft_rlimit(realm::util::Resource, long)

# intersect with nm build/oracle/trace_runner
  0
```

Zero of four. `evidence-and-linkage.md` category 1: **unreachable**.

> *Unreachable → park it. Do not port it. A green `make verify` on a unit the linker
> never pulls is not weak evidence, it is no evidence, and committing it records progress
> nothing can substantiate.*

These are `getrlimit`/`setrlimit` wrappers. Nothing in the `Storage` → `ObjectStore` →
`RealmFFIStatic` chain that `trace_runner` links calls them; they exist for tooling and
for tests. Porting this unit would produce ten green `make verify` runs and prove
nothing, because the C++ and the Rust would both be absent from the binary.

Note this is *not* the partial-linkage case seen in `util/compression` and
`util/demangle`, where live scaffolding masked a dead payload. Here nothing at all
survives, so the naive and realm-only filters agree and no refinement was needed.

## What would unblock it

A caller. Not a shim, not a trace — something in the linked library would have to invoke
`system_has_rlimit` or its siblings. Until then the unit is outside the harness's reach
by construction, and should stay C++.
