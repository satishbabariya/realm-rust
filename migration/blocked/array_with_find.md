# Parked: `upstream/src/realm/array_with_find.cpp`

83 lines of source, 92 exported realm symbols, all 92 linked, zero `throw`/`catch`/`try`.
Parked 2026-08-09 on **step 2** — and it is the narrowest step-2 park so far, which makes
it the **best available first customer for the vtable/RTTI shim**.

## "92 symbols" is the wrong number. The obligation is four.

The screen's `nm -g` count says 92, which reads as an enormous unit — 28 `find_optimized`
instantiations, 15 `find_sse` (SSE intrinsics), 10 `compare_relation`, 10
`compare_equality`, and so on, all instantiated from a 987-line header.

**88 of those are `weak external` and coalesced.** One sampled instantiation,
`find_optimized<Less, 16>`, has **3 definers** in `librealm.a`. Removing this
translation unit does not remove them from the binary, and does not oblige Rust to
provide them.

Splitting `nm -m` by linkage gives the real obligation — four strong symbols:

```
realm::ArrayWithFind::first_set_bit(unsigned int) const
realm::ArrayWithFind::first_set_bit64(long long) const
realm::ArrayWithFind::find(int, int64_t, size_t, size_t, size_t, QueryStateBase*) const
realm::ArrayWithFind::find_all(IntegerColumn*, int64_t, size_t, size_t, size_t) const
```

Three of the four are easy:

- `first_set_bit` is a de Bruijn table lookup with an explicit `INT_MIN` guard against
  the UB in `v & -v`. Pure arithmetic.
- `first_set_bit64` is two calls to it.
- `find(int cond, …)` is a six-way `if` chain dispatching to `find<Equal>`,
  `find<NotEqual>`, `find<Greater>`, `find<Less>`, `find<None>`, `find<NotNull>` — all
  weak, all bindable from Rust (the `translate_critical` experiment settled that
  `weak private external` and `weak external` symbols in an archive are linkable).

**This is the mirror of the `nm -u` lesson already in the rules.** `nm -u` makes a unit
look *less* dependent than it is, because inlining hides callees. `nm -g` makes a unit
look *bigger* than it is, because template instantiation adds symbols the unit does not
own. Both are fixed by asking about linkage rather than counting lines.

## The one blocker: `find_all` must build a polymorphic object

```cpp
void ArrayWithFind::find_all(IntegerColumn* result, int64_t value, size_t col_offset,
                             size_t begin, size_t end) const
{
    ...
    QueryStateFindAll state(*result);
    REALM_TEMPEX2(find_optimized, Equal, m_array.m_width, (value, begin, end, col_offset, &state));
}
```

`QueryStateFindAll<IntegerColumn>`'s vtable, typeinfo and typeinfo-name are `weak
external` with **definer count 1** — this TU is their only source in the archive. And
crucially:

```
nm -m librealm.a | grep __ZTVN5realm17QueryStateFindAllINS_13IntegerColumnEEE
  (no undefined references from any other object)
```

**Nothing outside this unit references that vtable.** It exists solely so `find_all` can
construct the local `state` object it hands to the search templates, which then call
`QueryStateBase`'s pure virtuals (`match(size_t)`, `match(size_t, Mixed)`,
`match_pattern`) on it.

So this is a sub-case of definer-count-1 worth naming, because "nobody else needs it"
invites the conclusion that it is free to drop:

| definer count 1, and… | why the shim is still needed |
|---|---|
| referenced by other TUs (`util/compression`) | they will fail to link without it |
| referenced only internally (**this unit**) | the unit itself constructs the object |

Either way the vtable must be synthesized; only the reason differs.

## Why this is the shim's best first customer

Compared with the other units waiting on vtable synthesis:

| unit | strong symbols to supply | vtables to synthesize | also needs |
|---|---|---|---|
| `util/basic_system_errors` | 2 | 1 (over a **libc++** base, slot order not pinned by realm) | `std::string` by value, `__cxa_guard` static |
| `util/compression` | ~20 | 7, four of them public ABI | exception shim; and its payload is dead |
| `impl/output_stream` | 3 | 1 | exception shim, `std::ostream::write` |
| **`array_with_find`** | **4, three of them trivial** | **1, entirely private to the unit** | **nothing else — no throws, no STL by value** |

It has no exceptions, no STL returned by value, a live payload, and a vtable whose only
consumer is inside the unit. If the vtable/RTTI shim is built, this is the unit that
proves it with the least confounding.

## What would unblock it

Synthesis of one C++ vtable + `typeinfo` + `typeinfo name` for a realm class deriving
from `QueryStateBase`, plus the ability to call `QueryStateBase`'s pure virtuals through
it. The vtable slot indices are measurable with the pointer-to-virtual-member-function
technique recorded in `migration/JOURNAL.md` under the `array_unsigned` entry.
