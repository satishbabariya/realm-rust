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

A hit means the object defines the symbol. It does **not** yet mean this is the class's
**key-function TU**, and only the key-function case is a park.

> **Corrected 2026-08-09 (reflection #5). A defined `ZT*` is not the test; the
> definer count is.** "Emitted here and nowhere else" holds only for a class with a key
> function — the first non-inline, non-pure virtual. Two common shapes have **no key
> function at all** and get `weak external` vtables emitted into *every* TU that needs
> them, which the linker then coalesces:
>
> - a class whose virtuals are all inline or pure (`ExceptionWithBacktraceBase`)
> - a template instantiation (`ExceptionWithBacktrace<std::invalid_argument>`)
>
> Removing such a TU orphans nothing and Rust would synthesize nothing.

So the second measurement, per symbol, is how many objects in the archive define it:

```
nm -m build/oracle/realm-core/src/realm/librealm.a | grep " <mangled-ZT-symbol>$" | grep -vc undefined
```

| Definers | Meaning | Verdict |
|---|---|---|
| 1 | sole source; removing the TU orphans the vtable | **park** — the shim must synthesize it |
| >1 | coalesced weak vtable, other TUs supply it | **not a park** |

`non-external` (anonymous-namespace) symbols are definer-count 1 by construction, so the
older local-vs-external distinction is a special case of this one and is dropped in
favour of it. **Do not read the `weak`/`external` attribute as the answer** —
`util/compression` defines four `weak external` vtables and is still the sole definer of
all four.

Measured:

| Unit | `ZT*` defined | definers | verdict |
|---|---|---|---|
| `util/time` #16 | 6 | **2 and 5** — one of five objects defining them | **not a step-2 park.** The screen sent it here and was wrong |
| `util/basic_system_errors` #8 | 3 × `non-external` | 1 | park stands |
| `util/compression` #14 | 19, incl. 4 × `weak external` | **1** each | park stands |

When the definer count is 1, porting the unit means synthesizing vtable, typeinfo and
typeinfo-name from Rust, with `__cxa_pure_virtual` filling the slots of pure virtuals.
Park unless the vtable/RTTI shim already exists — that shim is shared infrastructure,
not something to improvise inside one unit. Three units currently gate on it
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

**4. `nm -g $OBJ` — the full exported list, then split it by linkage.**

> **Corrected 2026-08-10. `nm -g` massively overstates the obligation.** The number that
> matters is how many symbols the object defines as **strong external** *and* is the sole
> definer of. Everything else — `weak external` template instantiations, and
> `weak private external` / `non-external` instantiations that every TU emits its own copy
> of — is supplied elsewhere, so removing this TU orphans nothing and Rust need not define
> it.
>
> Measured across the pending set, the gap is not marginal, it is the difference between
> "too big to attempt" and "an afternoon":
>
> | unit | `nm -g` | **strong** | lines |
> |---|---|---|---|
> | `link_translator` | 293 | **2** | 83 |
> | `impl/copy_replication` | 153 | **11** | 283 |
> | `to_json` | 113 | **8** | 515 |
> | `array_fixed_bytes` | 91 | **2** | 220 |
> | `history` | 45 | **1** | 279 |
> | `array_backlink` | 42 | **8** | 277 |
> | `array_decimal128` | 20 | **7** | 324 |
> | `node` | 16 | **8** | 170 |
>
> The command:
>
> ```
> nm -m $OBJ | grep -v '(undefined)' | grep -E '__ZN[A-Z]*5realm' \
>   | grep -v 'weak external' | grep -v 'non-external' | grep -v 'private external' \
>   | grep ' external ' | awk '{print $NF}' | sort -u
> ```
>
> then confirm each is sole-definer against `librealm.a`, exactly as step 2 does for `ZT*`.
> `array_backlink` is 8 strong symbols of which all 8 are sole-definer, and its 12 weak
> exports all have >1 definer — so the obligation is 8, not 42.
>
> This does **not** relax the all-or-nothing rule; it states it correctly. A TU is
> all-or-nothing in its **strong sole-definer** symbols, because those are what force the
> archive member to be extracted. Coalesced and per-TU-private instantiations never do.

Work from the exported list, never from the class declaration in the header. The Itanium ABI emits constructors and destructors in
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

**6. Non-trivial types by value in any signature.** `std::string`/`std::vector`
returned by value means an `sret` pointer and C++-allocator ownership rules. Possible —
`base64` did it — but never something to discover mid-port. Buffers handed back to C++
must come from `operator new`.

**Triviality decides the return class, not size.** Under the Itanium ABI, any type with
a user-provided destructor, copy constructor or move constructor is **MEMORY class
regardless of how small it is**: hidden result pointer in `rdi`, and the callee returns
that same pointer in `rax`. Two measured cases, in opposite directions:

| Returned type | Size | Trivial? | Passing |
|---|---|---|---|
| `std::optional<size_t>` (`util/base64`) | 16 B | yes | **registers** — `rax:dl` |
| `util::bind_ptr<ErrorInfo>` (`status`) | 8 B, one pointer | **no** — `~bind_ptr() { unbind(); }` | **`sret`** |

Declaring `bind_ptr` as an 8-byte struct return cost an hour on `status`: Rust returned
it in `rax`, so **every argument shifted by one register** and a small integer was
dereferenced as a `std::string*`. Size is not a shortcut here — grep the type's header
for a user-provided `~T`, `T(const T&)` or `T(T&&)` before writing the signature.

The cheapest confirmation is the oracle's own prologue, and it takes a minute:

```
6a: movq %rdx, %rbx     ; reason  = 3rd argument
6d: movl %esi, %r14d    ; code    = 2nd argument
70: movq %rdi, %r15     ; sret    = 1st argument
```

**7. Static-initialiser dependencies.** Check whether anything in the unit runs before
`main`, and whether the unit's globals are set by a static initialiser or by an explicit
call. `utilities.cpp`'s `cpuid_init()` is called from `group.cpp:47`; had it been a
static initialiser, removing the C++ TU would have left `sse_support`/`avx_support` at
`-1` and silently changed code paths across the library.

> **Corrected 2026-08-10.** Step 8's companion VTT check was written as
> `nm -u $OBJ | grep '^VTT for'`. **`nm -u` prints mangled names**, so that grep matched
> demangled text against mangled output and returned 0 for every object ever screened.
> The note that it was "validated across 15 units with zero false positives" was vacuous:
> it never fired once. The check is `grep '^__ZTT'`, or pipe through `c++filt` first:
>
> ```
> nm -u $OBJ | grep '^__ZTT'          # mangled, what nm actually emits
> nm -u $OBJ | c++filt | grep '^VTT'  # equivalent, slower
> ```
>
> Re-run corrected over all 67 `Storage` objects, **18 units reference a VTT** —
> `chunked_binary`, `db`, `global_key`, `mixed`, `obj`, `query`, `query_engine`,
> `query_expression`, `sort_descriptor`, `util/backtrace`, `util/bson/bson`,
> `util/encrypted_file_mapping`, `util/interprocess_condvar`, `util/serializer`,
> `util/terminate`, `util/to_string`, `util/uri`, `version`. **None is ported**, so the
> dead check never produced a wrong port; every unit it would have flagged was parked for
> some other reason or is still pending. It cost nothing this time, which is exactly why
> it survived fifteen screens.
>
> Two refinements the corrected run makes obvious:
>
> - **An undefined VTT is not automatically a park.** It is *undefined* here, i.e. libc++
>   or another TU supplies it; Rust does not have to synthesize it. What it signals is
>   that the unit **constructs an object with virtual bases inline**, so the constructor
>   was expanded into this TU and Rust would have to reproduce base-offset initialisation.
>   Check whether an out-of-line constructor can be bound instead.
> - **Most of these VTTs are for libc++ stream types, not realm classes.** A
>   `VTT for std::basic_stringstream` means "this unit builds a stringstream", which drags
>   in the whole iostream construction path — `ios_base::init`, `locale`, `use_facet`,
>   `ctype<char>::id`, `basic_ios::~basic_ios`, the `sentry` pair, and three stream
>   vtables. That is a much bigger surface than the symbol count suggests.

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

**A symbol-table read is never sufficient to park a unit — or to clear one.** That is the
whole section in one line, and it is the most-confirmed claim in this repo: eight
consecutive iterations have each parked, nearly parked, or wrongly cleared a unit on an
`nm` result that answered a narrower question than the step it served — one at step 1,
two at step 2, three at step 3, one at step 5, and one on a symbol *attribute* read in
the wrong artifact. Each was a different step, so each time the fix looked like "add a
row to the table", and the next iteration found a new row. The table below is still
worth having, but it is a list of instances, not the rule. The rule is the escalation:

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
| 1 `comm -12` over `grep '^__Z'` | how many mangled symbols survive to the link? | "8 of 14 linked, so it is reachable" | **filter to realm-owned symbols** (`^__ZN[A-Z]*5realm`). `version.cpp`'s 8 survivors are all libc++ weak helpers; **zero** `realm::Version` symbols link. The same filter keeps `util/enum` and `util/cli_args` correctly parked, which the naive count would have released |
| 2 `nm $OBJ \| grep -E '__ZT[VIS]'` | which `ZT*` does this object **define**? | "it defines one, so it is the key-function TU — park" | **count definers across `librealm.a`.** `util/time` defines six and shares every one with 1–4 other objects: no-key-function classes emit `weak external` vtables everywhere and the linker coalesces |
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
