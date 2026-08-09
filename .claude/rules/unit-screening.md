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

Let `OBJ = build/oracle/realm-core/src/realm/CMakeFiles/Storage.dir/<unit>.cpp.o`.

> **Corrected 2026-08-09 (reflection #4).** This file and `evidence-and-linkage.md` both
> said `build/oracle/CMakeFiles/Storage.dir/…`. That directory does not exist —
> realm-core is an `add_subdirectory`, so the objects are three levels down. Every
> screen command silently produced "no such file" and had to be re-derived by hand.

**1. Reachability.** Intersect `nm -g $OBJ | grep -v ' U '` with `nm build/oracle/trace_runner`.
Zero surviving symbols → park it; the gate cannot distinguish your port from an empty
file. Full treatment in `evidence-and-linkage.md`.

**2. Vtable and RTTI.** The question is which `ZT*` symbols this object **defines** —
not which it mentions, and not only the external ones:

```
nm $OBJ | grep -v ' U ' | grep -E '__ZT[VIS]'
```

Any hit means this is some class's **key-function TU**: the compiler emits the vtable,
typeinfo and typeinfo-name here and nowhere else, with `__cxa_pure_virtual` filling the
slots of pure virtuals. Porting it means synthesizing all three from Rust. Park unless
the vtable/RTTI shim already exists — that shim is shared infrastructure, not something
to improvise inside one unit. Three units currently gate on it
(`util/misc_ext_errors`, `util/basic_system_errors` #8, `obj_list` #15).

Both simpler forms of this command are wrong, in opposite directions, and both have
been used here:

| Form | Failure |
|---|---|
| `nm -g $OBJ \| grep __ZT` | misses **local** vtables. `-g` lists external symbols only, and an anonymous-namespace class emits its vtable and typeinfo as `non-external`. Reports zero for `util/basic_system_errors`, which defines three |
| `nm $OBJ \| grep -E "ZTV\|ZTI\|ZTS"` | **false-positives on references**. It also matches `U __ZTV…`, which only means the unit constructs a polymorphic object someone else owns. Six of the first 34 units trip this |

`error_codes` #13 was listed here as vtable-gated until 2026-08-09. Measured with the
form above it defines **no** `ZT*` — 16 mangled symbols, all 16 in the linked oracle.
It was never gated on the shim.

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

**5. Exceptions — `grep -cE '\b(throw|catch|try)\b'`, not `grep -c throw`.** Non-zero →
park, unless the exception shim exists. Throwing from Rust needs `__cxa_throw` with a
correctly built exception object and matching RTTI, the same class of work as step 2.
`uuid.cpp` (#24) was rejected here.

`catch` is the half that was missing and it is the harder half. `util/backtrace.cpp` has
**zero** `throw` and is unportable: `materialize_message()` is `noexcept` with its body
wrapped in `try { … } catch (...) { return msg; }`, which is load-bearing — it is how
the function does allocating, throwing work while promising not to throw. **Rust cannot
catch a foreign C++ exception**, and `panic = "abort"` closes the other door.

When the source grep says clean but `nm -u $OBJ` shows `___cxa_throw` or
`___cxa_allocate_exception`, do not park on the symbols. **Ask which function owns the
throw site:**

```
llvm-objdump -d -r $OBJ | awk '/^[0-9a-f]+ </{fn=$0} /___cxa_(throw|begin_catch|allocate_exception)/{print fn}' | sort -u
```

- Every owner is a libc++ weak helper (`std::__throw_length_error`,
  `std::__put_character_sequence`, `__throw_bad_array_new_length`) or
  `___clang_call_terminate` → the EH came in with an inlined `std::string`/`std::vector`
  instantiation or is a `noexcept` landing pad. Not a park. Its only trigger is a
  length/allocation failure, i.e. the same divergence `panic = "abort"` already
  documents. Confirmed on `error_codes`, `util/to_string`, `util/terminate`, `status`.
- A `realm::` function owns the site → real EH, park. Confirmed on `util/backtrace`,
  where the owner is `ExceptionWithBacktraceBase::materialize_message`.

The intermediate rule *"`__cxa_allocate_exception`/`__cxa_throw` are never
landing-pad-only, therefore park"* was drafted from `util/backtrace` alone and is
**false**: it would have parked `error_codes`, `util/to_string` and `util/terminate`,
all of which import both symbols and contain no reachable throw of their own.

**6. STL by value in any signature.** `std::string`/`std::vector` returned by value
means an `sret` pointer and C++-allocator ownership rules. Possible — `base64` did it —
but never something to discover mid-port. Buffers handed back to C++ must come from
`operator new`.

**7. Static-initialiser dependencies.** Check whether anything in the unit runs before
`main`, and whether the unit's globals are set by a static initialiser or by an explicit
call. `utilities.cpp`'s `cpuid_init()` is called from `group.cpp:47`; had it been a
static initialiser, removing the C++ TU would have left `sse_support`/`avx_support` at
`-1` and silently changed code paths across the library.

**8. Class-shaped units only: expand the inline methods the unit calls on its own
bases**, and find what those bottom out in. This is the one expensive step, and it is
the only one that reads headers rather than the object. Run it when steps 1–7 pass and
the unit is a class implementation.

Steps 1–7 are all `nm` on the unit's object, and **inlined base-class helpers are
invisible to every one of them**. `ArrayUnsigned::update_from_parent()` is six lines of
apparent field access; through `Node::get_ref_from_parent`, `ArrayUnsigned::init_from_ref`
and `Allocator::translate` it expands into two virtual dispatches
(`ArrayParent::get_child_ref`, `Allocator::do_translate`) and a call to
`Allocator::translate_critical`. Two separate iterations screened that unit clean and
concluded "field access at fixed offsets is all it needs"; both were wrong.

The step's output is not a verdict, it is a list. Each thing at the bottom is one of:

| Bottoms out in | Cost |
|---|---|
| arithmetic / bit-unpacking | reimplement in Rust |
| an out-of-line realm symbol | one `#[link_name]` import — cheap, see below |
| a virtual call | one measured vtable slot index — ~10 lines, not a shim |
| a C++ template driven by a lambda | park (`column_binary`) |

**Do not park because a symbol looks unreachable to the linker.** `weak private
external` (Mach-O `N_PEXT`) means external *during* static linking and made local *in
the output image*; the archive symbol table carries it and `ld` resolves references to
it from an object placed earlier on the link line — exactly where `librealm_core_rs.a`
sits. The lowercase `t` in `nm` on the linked binary is the *result* of that hiding, not
evidence the symbol was unavailable. `Allocator::translate_critical` was parked on this
misreading and un-parked after two three-line link probes settled it in five minutes.

## What each step does **not** tell you

**A symbol-table read is never sufficient to park a unit.** That is the whole section in
one line, and it is the most-confirmed claim in this repo: six consecutive iterations
have each parked, or nearly parked, a unit on an `nm` result that answered a narrower
question than the step it served — one at step 2, three at step 3, one at step 5, and
one on a symbol *attribute* read in the wrong artifact. Each was a different step, so
each time the fix looked like "add a row to the table", and the next iteration found a
new row. The table below is still worth having, but it is a list of instances, not the
rule. The rule is the escalation:

> When a screen step is about to decide port-or-park, take a **second measurement of a
> different kind** — one that does not read a symbol table. Dump the record layout, read
> the flags, disassemble the owning function, or link a probe. Each of the six took
> under ten minutes; two of them reversed the verdict, and one of those had already
> shipped a wrong park file and a wrong recommendation to the user twice.

The general form of why this keeps happening: **`nm` describes what survived to the
link, and inlining, macro expansion, `noexcept` landing pads and `N_PEXT` hiding all
move code and visibility across that boundary without changing what the unit does.**

Instances, each confirmed on a named unit:

| Step | Narrow question it answers | Wrong conclusion | Disambiguate with |
|---|---|---|---|
| 2 `grep ZTV\|ZTI\|ZTS` | is this the **key-function TU**? | "the class is not polymorphic, so the layout is what the header says" | `clang -Xclang -fdump-record-layouts`. `array_unsigned.cpp.o` has no `ZT*` symbol, yet `Node` has a vptr at offset 0 and every field is shifted 8 bytes |
| 3 `nm -u` (short list) | what must the **linker** still resolve? | "few dependencies, therefore self-contained" | read the body. Templates, lambdas passed to templates, and inline members of other realm classes were compiled in, not linked — they leave `nm -u` entirely (`column_binary.cpp`: 4 undefined symbols, unportable) |
| 3 `nm -u` shows `util::terminate` | does any **unconditional** assert survive? | "assertions are live in this build" | `assert.hpp` + the cache. `REALM_ASSERT_RELEASE` and `REALM_UNREACHABLE()` are under no `#if` and always call `terminate`; `REALM_ASSERT*` are separately gated. 42 of 67 `Storage` objects reference `terminate` |
| 3 `nm -u` shows `__cxa_begin_catch`, `__gxx_personality_v0` | is there **EH machinery**? | "this unit throws, park it per step 5" | **which function owns the site** — `llvm-objdump -d -r`, see step 5. `array_unsigned.cpp` has both symbols and zero `throw`; so do `error_codes`, `util/to_string`, `util/terminate` with `___cxa_throw` on top, and all four are clean |
| 2 `nm -g … \| grep __ZT` | which **external** `ZT*` are here? | "no vtable is emitted" | drop `-g`, filter `U`. `util/basic_system_errors` emits three `ZT*` as `non-external` from an anonymous-namespace `std::error_category` subclass |
| any step, on a **linked binary** | what did the image end up with? | "`t` means local means Rust cannot bind to it" | link a probe. `weak private external` in the `.o` and `t` in the image are the same symbol at two stages of one process; the second is *caused by* the first, not a contradiction of it |

Step 2's first failure is the expensive one: an empty `ZT*` result means no vtable has to
be *synthesized*, which is a real and useful answer, but says nothing about whether the
objects the unit manipulates have a vptr in them. Those are different questions and the
second one silently shifts every field offset in the Rust view.

Step 3's failures are the same shape from opposite directions — a symbol's absence and a
symbol's presence both mean less than they look like.

> **Corrected 2026-08-09 (reflection #4).** Row 4 previously named `grep -c throw` as the
> disambiguator and asserted that `__cxa_begin_catch` + `__gxx_personality_v0` is a
> landing pad. **The conclusion is right for `array_unsigned` and stays; the test is too
> weak and is replaced.** A source grep for `throw` misses `catch`, which is what makes
> `util/backtrace` unportable, and it also cannot tell an inlined libc++ throw helper
> from a real one. Only the owning-function test separates all five measured cases.

### The inlining question is not binary

"Does this unit inline templates?" is the wrong test — nearly every unit in realm does.
The test that separates the two cases seen so far is **what** was inlined:

- `column_binary.cpp` inlined B+-tree traversal driven by a lambda handed to a C++
  function template. Rust cannot express that against a template, and cannot call it.
  **Parked.**
- `array_unsigned.cpp` inlines `realm::lower_bound<0|1|2|4>` — bit-unpacking arithmetic
  from `array_direct.hpp`. Rust reimplements it directly. **Portable.**

Ask: *can Rust reimplement what was inlined, or must it call it?* Only the second is a
park.

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
