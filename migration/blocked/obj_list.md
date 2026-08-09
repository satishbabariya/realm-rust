# Parked: `upstream/src/realm/obj_list.cpp`

Queue position #15 (depth 1, inbound 4, 25 lines). Parked 2026-08-09.

**Parked for the same reason as the `std::error_category` units, which is not visible
from the source.** This is a 25-line file whose entire body is:

```cpp
ObjList::~ObjList() {}
```

Live and reachable — this is not a deadness park.

## What the object actually exports

```
_ZN5realm7ObjListD0Ev                    realm::ObjList::~ObjList()   [deleting]
_ZN5realm7ObjListD1Ev                    realm::ObjList::~ObjList()   [complete]
_ZN5realm7ObjListD2Ev                    realm::ObjList::~ObjList()   [base]
_ZNK5realm7ObjList14get_owning_objEv     ObjList::get_owning_obj() const
_ZNK5realm7ObjList18get_owning_col_keyEv ObjList::get_owning_col_key() const
_ZNK5realm7ObjList7matchesERKS0_         ObjList::matches(ObjList const&) const
_ZTVN5realm7ObjListE                     vtable for realm::ObjList
_ZTIN5realm7ObjListE                     typeinfo for realm::ObjList
_ZTSN5realm7ObjListE                     typeinfo name for realm::ObjList
```

with `__cxa_pure_virtual` and `__class_type_info` undefined.

`ObjList` has a virtual destructor, so this translation unit is its **key function TU**:
the compiler emits the class's vtable and RTTI here and nowhere else. Removing it from
the hybrid removes the vtable for a polymorphic base class that other units instantiate
subclasses of.

## Why that is a park and not a port

Porting this unit means synthesizing from Rust:

- `_ZTVN5realm7ObjListE` — the vtable, including slots that must point at
  `__cxa_pure_virtual` for the pure virtual members;
- `_ZTIN5realm7ObjListE` — an `abi::__class_type_info` (not `__si_class_type_info`;
  `ObjList` has no base), which must interoperate with libc++'s RTTI for `dynamic_cast`
  and for the `catch` machinery;
- `_ZTSN5realm7ObjListE` — the mangled type-name string the type_info points at;
- three destructor variants, of which `D0` must call `operator delete`.

That is the same machinery `util/misc_ext_errors.cpp` was parked for, and the same
machinery `util/basic_system_errors.cpp` (#8) and `error_codes.cpp` (#13) need. It is a
deliberate piece of shared infrastructure, not something to improvise inside one unit.

`ObjList` is arguably the *cleanest* place to build it first — `__class_type_info` with
no base is the simplest RTTI shape, and the destructors are empty — so if the shim is
ever built, this is a good first customer.

## The screening lesson

**Line count is not a proxy for ABI difficulty.** The screen that has been working
(live symbols → `grep -c throw` → STL-by-value returns) passes this file on every
count: 25 lines, zero `throw`, no STL in any signature. It is nonetheless one of the
harder units in the queue.

The missing check is: **does the object emit `_ZTV`/`_ZTI`/`_ZTS`?** One `nm` over the
object answers it, and it should run before reading the source at all:

```
nm <unit>.o | grep -E "ZTV|ZTI|ZTS"
```

Any hit means the unit is the key-function TU for a polymorphic class and carries its
vtable and RTTI. Added to the screening order in `migration/JOURNAL.md`.

## What I would need to continue

The `std::error_category`-style vtable/RTTI shim, built deliberately and with its own
differential. That decision now gates four units — this one, `misc_ext_errors` (parked),
`basic_system_errors` (#8), and `error_codes` (#13) — which is a stronger argument for
building it once than any of them made individually.

Did not touch a trace, the comparator, the Makefile, or `upstream/`.

## Audit 2026-08-09 (reflection #2)

Still blocked; parked earlier the same day, so nothing has changed underneath it.

Recorded here so the next `/port-unit` sees it: **this is the only member of the
vtable/RTTI group that is both live and cheap.** `misc_ext_errors` is unreachable and
cannot be gated at all; `basic_system_errors` (#8) and `error_codes` (#13) are live but
carry `std::error_category` subclassing on top of the vtable work. If the shim is built,
build it here.

The screening lesson from this entry has been promoted out of the journal into
`.claude/rules/unit-screening.md`, which now carries the full ordered screen (the
vtable check is step 2, before the source is read).
