# Parked: `upstream/src/realm/array_unsigned.cpp`

271 lines, 10 exported methods, all reachable, byte-visible, no vtable to synthesize,
no `throw`, no STL returned by value. Parked 2026-08-09 on **ABI-shim** grounds, not
reachability.

**This reverses the "portable, and the right next unit" verdict of 2026-08-09.** That
entry is not withdrawn — everything it says about the unit's *own* dependencies is
still true. What it missed is one level down, and is stated here so the next reader
does not have to find it again.

## What the earlier screening got wrong

The scoping entry concluded:

> The Rust side must never construct or destroy these objects, only operate on a `this`
> pointer supplied by C++. With a vptr at offset 0 that is not a limitation worth
> fighting: **field access at fixed offsets is all the ported methods need.**

Field access is not all they need. `ArrayUnsigned::update_from_parent()` — six lines —
expands through inline header code into **two virtual dispatches and one inline
accessor that has no linkable definition**:

```cpp
void ArrayUnsigned::update_from_parent() noexcept          // array_unsigned.cpp:83
{
    ArrayParent* parent = get_parent();                    // field read, fine
    ref_type new_ref = get_ref_from_parent();              // -> m_parent->get_child_ref(ndx)   VIRTUAL
    init_from_ref(new_ref);                                // -> m_alloc.translate(ref)
}                                                          //    -> translate_critical()  no linkable symbol
                                                           //    -> do_translate()        VIRTUAL
```

The screen never looked at this because steps 1–7 all run on
`array_unsigned.cpp.o`, and inlined code is invisible to every one of them. This is the
same failure mode `unit-screening.md` already documents for `nm -u` — *"`nm` describes
what survived to the link, and inlining moves code across that boundary"* — appearing
in a place the rule does not yet point at: **the unit's own inline base-class helpers**,
not its callees.

## The three blockers, measured

### 1. `Allocator::translate_critical` has no linkable definition

`alloc.hpp:547`, `inline`. In `array_unsigned.cpp.o`:

```
nm -m .../array_unsigned.cpp.o
0000000000001310 (__TEXT,__text) weak private external __ZNK5realm9Allocator18translate_criticalEPNS0_14RefTranslationEm
```

**35** objects in `librealm.a` carry a definition, and **all 35** are
`weak private external` (`.private_extern`, hidden visibility) — checked, not assumed.
In the linked oracle it collapses to a **local** symbol:

```
nm build/oracle/trace_runner | grep translate_critical
000000010001aa10 t realm::Allocator::translate_critical(...) const      <- lowercase t
```

By contrast `translate_less_critical` has exactly **one** definition in the archive and
it is `T`. So Rust can bind to the slow path and not the fast one, which is the wrong
way round.

Reimplementing it in Rust is possible but is not this unit's work. It requires
`Allocator::RefTranslation`'s layout — which is **conditional on
`REALM_ENABLE_ENCRYPTION`** (`alloc.hpp:184-188`; `ON` in this build, so two extra
`EncryptedFileMapping*` members are present) — three atomic loads with specific
orderings, `encryption_read_barrier`, and `get_byte_size_from_header`. That is a copy of
`alloc.hpp`'s hot path living inside one array unit, and **every future unit that
touches a ref needs the same copy**. That is the definition of shared infrastructure.

### 2. Two virtual dispatches, slot indices measured

Neither `-Xclang -fdump-vtable-layouts` on this TU nor on a probe subclass emits
anything — this TU emits no vtables, which is the same fact as "not the key-function
TU". Measured instead from the Itanium encoding of a pointer-to-virtual-member-function
(odd `ptr` = 1 + byte offset into the vtable):

| Function | vtable byte offset | slot index | this-adjustment |
|---|---|---|---|
| `ArrayParent::get_child_ref(size_t) const` | 16 | **2** | 0 |
| `ArrayParent::update_child_ref(size_t, ref_type)` | 24 | 3 | 0 |
| `Allocator::do_translate(ref_type) const` | 48 | **6** | 0 |
| `Allocator::do_alloc(size_t)` | 24 | 3 | 0 |

Both needed slots have `adj = 0`, so the call is a plain
`(*(fn*)(*(void**)this + off))(this, args)`. Mechanically easy; the risk is not the
call, it is that the index is a hand-copied constant derived from declaration order in
a header this port does not own. Nothing in the build would catch it drifting — a wrong
slot is a call to the wrong allocator method, i.e. heap corruption, not a byte diff.

### 3. `Allocator`'s private field offsets (not a blocker, recorded for the shim)

From `-Xclang -fdump-record-layouts`:

```
*** class realm::Allocator                              [sizeof=64, align=8]
  0 | (Allocator vtable pointer)
  8 | atomic<size_t>          m_baseline               <- is_read_only(ref) is `ref < m_baseline`
 16 | ref_type                m_debug_watch
 24 | atomic<RefTranslation*> m_ref_translation_ptr    <- translate()'s fast/slow branch
 32 | atomic<uint64_t>        m_content_versioning_counter
 40 | atomic<uint64_t>        m_storage_versioning_counter
 48 | atomic<uint64_t>        m_instance_versioning_counter
 56 | bool                    m_is_read_only
```

`realm::MemRef` is `{char* m_addr; size_t m_ref}`, `sizeof = 16`, trivially copyable —
so `create_node` returns it in `rax:rdx`, no `sret`.

## Why not port the other nine methods and leave this one

Because the hybrid excludes C++ **purely by link order** (see
`.claude/rules/evidence-and-linkage.md`). If Rust defines 9 of the 10 symbols,
`update_from_parent` is still undefined when `librealm.a` is scanned, `ld` pulls
`array_unsigned.cpp.o` to resolve it, and the other nine become duplicate symbols. The
link fails. **Partial ports of a translation unit are not possible under this
mechanism** — it is all ten or none. Worth stating once here, because it is a general
property of the harness and it constrains every unit, not just this one.

## What would unblock it

A **ref-translation and virtual-dispatch shim**, in the crate rather than in a unit,
providing:

1. `translate(&Allocator, ref) -> *mut c_char` — the `translate_critical` fast path
   reimplemented over a `#[repr(C)]` `RefTranslation` built for the *configured*
   `REALM_ENABLE_ENCRYPTION`, falling back to the exported
   `translate_less_critical`, and to `do_translate` through the vptr when
   `m_ref_translation_ptr` is null.
2. `is_read_only(&Allocator, ref) -> bool` — `ref < m_baseline`, relaxed load.
3. A typed virtual-call helper so slot indices live in one audited table with the
   measurements above, not scattered as literals through unit ports.

All three are needed by essentially every array unit, so the shim is not overhead spent
on `array_unsigned` — it is the thing that makes the array units portable at all. The
whole-population screen (`128101c`) already put 78 of the 97 remaining units behind a
vtable shim; this adds the ref-translation half of the same boundary and identifies the
first concrete customer.

Once that exists, this unit is genuinely straightforward: the remaining Rust is node
header arithmetic, `get_direct`, `lower_bound<0|1|2|4>` / `upper_bound<0|1|2|4>` bit
unpacking, `std::lower_bound`/`upper_bound` over widths 8/16/32/64, and calls to the
already-exported `Node::create_node`, `Node::do_copy_on_write` and `Node::alloc`.

## Things measured this iteration that stay true regardless

- All 12 defined symbols of `array_unsigned.cpp.o` are present in
  `build/oracle/trace_runner`. Reachable, and byte-visible — it would have been the
  first unit `make diff-test` could genuinely judge.
- Of those 12, only 10 are `ArrayUnsigned` methods. `realm::get_direct`,
  `Allocator::translate_critical`, the `util::terminate<...>` instantiation and
  `___clang_call_terminate` are weak/coalesced and supplied by 4–36 other objects, so
  removing this TU does not orphan them.
- `bit_width`, `_set` and `_get` are private `inline` members with no emitted symbol —
  fully inlined, so Rust reimplements them rather than binding to them.
- Two live `REALM_UNREACHABLE()` calls at lines 120 and 158 must become
  `realm::util::terminate("Unreachable code", file, line)`, not a Rust panic and not
  `unreachable_unchecked` (`assert.hpp:99`; it is under no `#if`).
