# Parked: `upstream/src/realm/util/demangle.cpp`

48 lines, one function. Parked 2026-08-09 on **step 5 (real exceptions)** and on
**reachability of the payload** — and, like `util/time`, explicitly *not* on step 2.

## The linked subset excludes the function the unit is named for

Step 1 with reflection #5's realm-only filter reports 5 of 6 symbols linked, which reads
as "almost fully reachable". The one that is **not** linked is
`realm::util::demangle(const std::string&)` — the entire payload:

```
nm -g $OBJ | grep -v ' U ' | grep demangle
  0000000000000000 T realm::util::demangle(std::string const&)

nm build/oracle/trace_runner | grep -c 5realm4util8demangle
  0
```

The five that *are* linked are all `ExceptionWithBacktrace<std::bad_alloc>` scaffolding:
its default constructor, both destructor variants, `what()` and `message()`. Those exist
because the unit instantiates the template, not because anything calls `demangle`.

So the gate could not distinguish a correct port of this unit from an empty one. That is
category-1 unreachable applied to the part that matters.

**This is the second confirmed occurrence of the pattern**, after `util/compression` #14
(12/34 linked, none of them a compression function). Two occurrences is this project's
bar for promoting a rule, so it is now ready for `unit-screening.md` rather than living
in two park files:

> **Step 1 addendum.** When the linked subset is a strict subset, check whether it
> contains the functions the unit is *named for*. A high ratio is not the test; a unit
> whose payload is dead and whose exception or accessor scaffolding is live reads as
> "mostly reachable" and is worth nothing to the gate.

Mechanically: intersect the linked set with the unit's *primary* exports (the ones whose
mangled names contain the unit's own namespace-qualified function names), not with every
realm symbol it happens to define.

## Step 5 — a real throw, owned by the function itself

```cpp
case -1:
    REALM_ASSERT(!unmangled_name);
    throw util::bad_alloc{};
```

One `throw` in source, and the owning-function test puts the EH site in
`realm::util::demangle` itself:

```
llvm-objdump -d -r $OBJ | awk '/^[0-9a-f]+ </{fn=$0} /___cxa_(throw|begin_catch|allocate_exception)/{print fn}' | sort -u
realm::util::demangle(std::string const&)
realm::util::demangle(std::string const&) (.cold.1)
std::__1::__throw_length_error[abi:nqe210106](char const*)    <- libc++ helper, ignore
```

`util::bad_alloc` is `ExceptionWithBacktrace<std::bad_alloc>`, so throwing it from Rust
needs `__cxa_allocate_exception`, a constructed exception object carrying a `Backtrace`,
matching RTTI — and `Backtrace::capture()`, which lives in the parked
`util/backtrace.cpp`. Same transitive block as `util/time`.

## Step 2 does **not** apply

Six defined `ZT*`, and under reflection #5's definer-count rule none of them parks it:

| symbol | definers |
|---|---|
| `vtable`/`typeinfo`/`typeinfo name` for `ExceptionWithBacktrace<std::bad_alloc>` | 2 |
| `vtable`/`typeinfo`/`typeinfo name` for `detail::ExceptionWithBacktraceBase` | 5 |

All coalesced; removing this TU orphans nothing. Third unit in a row where the naive
step-2 reading would have parked for a reason that does not exist.

## What would unblock it

The exception-construction shim plus `Backtrace::capture()`. But item 1 above makes that
moot for this unit specifically: even with both shims, `demangle` is not linked, so
porting it would be unmeasurable. It should stay C++ unless something starts calling it.

`demangle` exists to prettify type names in error messages and assertion output. Like
`util/backtrace`, its cost is high and its byte-identity benefit is zero.
