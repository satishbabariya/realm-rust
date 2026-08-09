---
description: The ordered screen that decides whether to port a unit, run before reading its source. Derived from four consecutive sessions that each rejected a candidate late and each added a missing step.
paths:
  - "crates/**/*.rs"
  - "migration/**"
---

# Screening a candidate unit

`evidence-and-linkage.md` answers "can any check here see this unit?". This file
answers the question after it: "is this unit's ABI surface something I can supply?"

**Line count is not a proxy for difficulty.** `obj_list.cpp` is 25 lines whose entire
body is `ObjList::~ObjList() {}`, exports nine symbols, and is one of the harder units
in the queue. `utilities.cpp` is larger and was routine. The queue orders by lines and
dependency depth; neither correlates with ABI cost.

Run the whole screen **before reading the source**. Steps 1–4 are `nm` on one object
and cost about a second each. Three of the last five units examined were rejected at
step 2, 5 or 6 — every one of those rejections came after source had already been read,
which is the waste this ordering exists to prevent.

## The order

Let `OBJ = build/oracle/CMakeFiles/Storage.dir/<unit>.cpp.o`.

**1. Reachability.** Intersect `nm -g $OBJ | grep -v ' U '` with `nm build/oracle/trace_runner`.
Zero surviving symbols → park it; the gate cannot distinguish your port from an empty
file. Full treatment in `evidence-and-linkage.md`.

**2. Vtable and RTTI.**

```
nm $OBJ | grep -E "ZTV|ZTI|ZTS"
```

Any hit means this is some class's **key-function TU**: the compiler emits the vtable,
typeinfo and typeinfo-name here and nowhere else, with `__cxa_pure_virtual` filling the
slots of pure virtuals. Porting it means synthesizing all three from Rust. Park unless
the vtable/RTTI shim already exists — that shim is shared infrastructure, not something
to improvise inside one unit. Four units currently gate on it (`util/misc_ext_errors`,
`util/basic_system_errors` #8, `error_codes` #13, `obj_list` #15).

**3. `nm -u $OBJ` — size of the undefined set.** This also picks the differential shape;
see `evidence-and-linkage.md`. A short, self-contained list is a good sign in itself.

**4. `nm -g $OBJ` — the full exported list.** Work from this, never from the class
declaration in the header. The Itanium ABI emits constructors and destructors in
variants — `C1` complete / `C2` base, `D0` deleting / `D1` complete / `D2` base — and
the compiler emits *all* of them even when they are behaviourally identical. Defining
only `C1`/`D1` links fine until some caller references another form.
`SemaphoreMutex` has three methods in the header and seven symbols in the object.

Delegate the variants to one shared `construct`/`destruct` rather than writing each
twice.

**5. `grep -c throw`.** Non-zero → park, unless the exception shim exists. Throwing from
Rust needs `__cxa_throw` with a correctly built exception object and matching RTTI, the
same class of work as step 2. `uuid.cpp` (#24) was rejected here.

**6. STL by value in any signature.** `std::string`/`std::vector` returned by value
means an `sret` pointer and C++-allocator ownership rules. Possible — `base64` did it —
but never something to discover mid-port. Buffers handed back to C++ must come from
`operator new`.

**7. Static-initialiser dependencies.** Check whether anything in the unit runs before
`main`, and whether the unit's globals are set by a static initialiser or by an explicit
call. `utilities.cpp`'s `cpuid_init()` is called from `group.cpp:47`; had it been a
static initialiser, removing the C++ TU would have left `sse_support`/`avx_support` at
`-1` and silently changed code paths across the library.

## Layout is not the port's to choose

For any class-shaped unit, the member layout is fixed by the header that every *other*
TU still compiles against. Rust has no freedom there — it accepts whatever the header
says. Prefer units whose members are simple: `SemaphoreMutex` is one pointer, so `this`
is `*mut *mut c_void` and there is nothing to get wrong.

Corollary: `realm::util::Span<T, N>` with a fixed extent stores **only a pointer**
(`util/span.hpp:262`); dynamic-extent `Span<T>` stores `{ptr, size}`. Getting this
backwards shifts every subsequent argument register, and it is invisible to the type
system on both sides.

## Screen the whole queue at once, not the head of it

Classifying only the cheapest-to-reach units and generalising from them cost five ticks
once: seven of the thirteen depth-0 units are dead, but only five of the other
forty-seven are, because depth-0 leaves are disproportionately sync-only helpers and
standalone-tool utilities. A run over the head of the queue is not a sample of the
queue.
