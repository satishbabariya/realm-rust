# Parked: `upstream/src/realm/util/basic_system_errors.cpp`

111 lines, 2 real exported functions, all 5 defined symbols reachable in
`build/oracle/trace_runner`. Parked 2026-08-09 on the **vtable/RTTI shim**, which is
what `unit-screening.md` step 2 says to park for.

Reachability is not the problem here — unlike the seven `REALM_ENABLE_SYNC` parks, this
unit is genuinely live.

## The two functions, and what is behind them

```cpp
namespace {
class system_category : public std::error_category {
    const char* name() const noexcept override;      // "realm.basic_system"
    std::string message(int) const override;         // strerror_r into a 257-byte buffer
};
}

namespace realm::util::error {
std::error_code make_error_code(basic_system_errors err) noexcept
{
    return std::error_code(err, basic_system_error_category());
}
const std::error_category& basic_system_error_category()
{
    static system_category system_category;          // function-local static
    return system_category;
}
}
```

Two exported symbols to supply (the other three are weak libc++ `__throw_length_error`
helpers, coalesced from many objects). Behind them, Rust would have to synthesize:

1. **A vtable for a `std::error_category` subclass.** Seven slots — `~D1`, `~D0`,
   `name()`, `message(int)`, and three inherited-but-virtual slots that must point at
   libc++'s own implementations, which the object leaves undefined and therefore
   linkable:
   ```
   std::__1::error_category::default_error_condition(int) const
   std::__1::error_category::equivalent(int, std::__1::error_condition const&) const
   std::__1::error_category::equivalent(std::__1::error_code const&, int) const
   std::__1::error_category::~error_category()
   ```
   The slot order is libc++'s, not realm's, and nothing in this repo pins it.
2. **RTTI.** `typeinfo` as a `__si_class_type_info` (single inheritance) whose base
   pointer is `typeinfo for std::__1::error_category`, plus the `typeinfo name` string,
   plus a reference to `vtable for __cxxabiv1::__si_class_type_info` — all three appear
   in the object exactly as that shape.
3. **`std::string` returned by value** from `message(int)` — sret plus libc++'s
   short-string-optimisation layout plus `operator new` ownership. Screen step 6. On its
   own this is precedented (`base64` returned `std::vector<char>`); stacked on 1 and 2 it
   is not a detail.
4. **A thread-safe function-local static** — `__cxa_guard_acquire` / `__cxa_guard_release`
   around first use and `__cxa_atexit` to register the destructor. The object imports all
   three and a `guard variable for ...::system_category` symbol.

## Why the screen said "no vtable" and was wrong

Step 2 of `unit-screening.md` is the right test; the way I ran it was not. The three
`ZT*` symbols this TU defines are **local**, because the class is in an anonymous
namespace:

```
nm -m .../util/basic_system_errors.cpp.o | grep __ZT | grep -v ' U '
(__DATA,__const)  non-external  typeinfo for (anonymous namespace)::system_category
(__TEXT,__const)  non-external  typeinfo name for (anonymous namespace)::system_category
(__DATA,__const)  non-external  vtable for (anonymous namespace)::system_category
```

`nm -g` lists only external symbols, so a screen built on `nm -g` reports **zero** ZT*
here and calls the unit a candidate. The rule's own formulation (`nm $OBJ`, no `-g`)
does catch these — but it has the opposite fault: it also matches `U __ZTV…`
*references*, which merely mean the unit constructs a polymorphic object and say nothing
about who emits the vtable. Six units in this queue trip that false positive.

The test that is right in both directions is **ZT\* that the unit defines, local or
external**:

```
nm $OBJ | grep -v ' U ' | grep -E '__ZT[VIS]'
```

Third entry in the family of `nm` over-readings already tabulated in
`unit-screening.md`. Proposed there for reflection #4 rather than edited in from a port.

## Correction to a claim in `unit-screening.md`

That file currently says four units gate on the vtable shim: `util/misc_ext_errors`,
`util/basic_system_errors`, **`error_codes` (#13)** and `obj_list` (#15). Re-run with the
corrected test, `error_codes.cpp` defines **no** `ZT*` at all — 17 symbols, all 17
linked, no vtable, no `throw`. It is a candidate, not a shim customer. `status.cpp`,
`object_id.cpp`, `util/to_string.cpp`, `util/backtrace.cpp` and `util/terminate.cpp`
likewise come out clean on this test.

## What would unblock it

The same vtable/RTTI shim named by `obj_list` and by the whole-population screen
(`128101c`), with one addition this unit is the first to need: the vtable being
synthesized derives from a **libc++** class, not a realm one, so the shim needs a way to
pin `std::error_category`'s virtual slot order to a measured value rather than a
hand-copied one. The pointer-to-virtual-member-function technique recorded in
`migration/blocked/array_unsigned.md` measures exactly that and works on protected and
inherited virtuals.
