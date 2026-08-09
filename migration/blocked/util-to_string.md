# Parked: `upstream/src/realm/util/to_string.cpp`

151 lines, 7 realm symbols, all linked, live payload. **Passes every step of
`unit-screening.md`** — zero `ZT*` defined, zero `throw`/`catch`/`try`, every EH site
owned by a libc++ weak helper — and it is not portable. Parked 2026-08-09.

## The blocker: it *constructs* C++ objects with virtual bases

```cpp
std::string Printable::str() const {
    std::ostringstream ss;                  // <-- constructs a virtual-base class
    ss.imbue(locale_classic);
    ss.exceptions(std::ios_base::failbit | std::ios_base::badbit);
    print(ss, true);
    return ss.str();
}

std::string format(const char* fmt, std::initializer_list<Printable> values) {
    std::stringstream ss;                   // <-- same
    ss.imbue(locale_classic);
    format(ss, fmt, values);
    return ss.str();
}
```

`nm -u` names the consequence directly:

```
VTT for std::__1::basic_stringstream<char, ...>
VTT for std::__1::basic_ostringstream<char, ...>
vtable for std::__1::basic_streambuf<char, ...>
vtable for std::__1::basic_stringbuf<char, ...>
vtable for std::__1::basic_stringstream<char, ...>
vtable for std::__1::basic_ostringstream<char, ...>
```

A **VTT** (virtual table table) exists only for classes with **virtual bases**.
iostreams has them: `basic_istream` and `basic_ostream` both virtually inherit
`basic_ios`. Constructing such an object is not "call the constructor" — it is
participating in C++'s virtual-base construction protocol: the constructor consumes a
VTT and initialises several vtable pointers plus a virtual-base offset table in a
specific order. Reproducing that from Rust is not an ABI detail, it is reimplementing a
compiler feature.

Everything *else* the unit needs is callable: `ostream::operator<<(double)`,
`operator<<(long)`, `operator<<(unsigned long)` and `ostream::write` are out-of-line
members; `__put_character_sequence`, `__pad_and_output` and `__quoted_output` are weak
instantiations. If the two `std::stringstream` locals were passed in from C++ instead of
constructed here, this unit would be routine.

## A second, independent problem

```cpp
namespace { std::locale locale_classic = std::locale::classic(); }
```

`__GLOBAL__sub_I_to_string.cpp` plus `___cxa_atexit`. Unlike `object_id`'s random
generator state — where lazy initialisation was unobservable because every consumer was
random anyway — **this value is observable**: it is imbued into every stream and decides
number formatting. A Rust port would have to construct and own a `std::locale` (a
refcounted handle) and register its destructor.

## The screening gap this exposes

None of steps 1–7 detects this. The unit is reachable, defines no vtable, and its source
contains no `throw`, `catch` or `try`. **Proposed as a new step, tested in both
directions across 15 units before proposing:**

> **Step 8. `nm -u $OBJ | grep '^VTT for'`.** A `VTT for X` reference means the unit
> constructs an object of class `X`, and `X` has virtual bases. Constructing one from
> Rust means implementing C++'s virtual-base construction protocol. Park unless a
> virtual-base construction shim exists.

| VTT count | units | outcome |
|---|---|---|
| 0 | all **10 ported** units, plus `util/compression`, `uuid`, `table_ref` (parked for other reasons) | no false positives |
| ≥1 | `util/to_string` (2), `util/backtrace` (1) | exactly the two whose blocker is iostreams construction |

`util/backtrace`'s park file already named `std::stringstream` construction as a
blocker; this is the second occurrence, which is the promotion bar, and the VTT grep
turns a judgement call into one command.

## What would unblock it

A fourth shim, distinct from the three already characterised: constructing C++ objects
with virtual bases. That is the least attractive of the four — the other three are ABI
layers, this one reimplements a language feature — and the better answer for this unit
is probably that it stays C++.
