# Parked: `upstream/src/realm/util/backtrace.cpp`

175 lines, 11 real exported symbols, all 20 defined symbols reachable in
`build/oracle/trace_runner`. Defines no vtable. `grep -c throw` returns **0**. It passes
every step of `.claude/rules/unit-screening.md` and it is not portable.
Parked 2026-08-09. **Third consecutive park — this one trips the loop's stop condition.**

## Most of the unit is genuinely easy

`Backtrace` is `{void* m_memory; char* const* m_strs; size_t m_len}` — 24 bytes. Its
destructor, both copy/move constructors (`C1`/`C2` variants), both assignment operators
and `capture()` are `malloc`/`free`/`strlen`/`memcpy` plus two libSystem calls
(`::backtrace`, `::backtrace_symbols`). `capture()` returns `Backtrace` by value → sret.
All of that is a routine port.

## The two functions that block it

### `materialize_message()` — a `catch (...)` Rust cannot express

```cpp
const char* detail::ExceptionWithBacktraceBase::materialize_message() const noexcept
{
    if (m_has_materialized_message) return m_materialized_message.c_str();
    const char* msg = message();                       // PURE VIRTUAL call
    try {
        std::stringstream ss;                          // full iostreams object
        ss << msg << "\n";
        ss << "Exception backtrace:\n";
        m_backtrace.print(ss);
        m_materialized_message = ss.str();             // std::string by value, assigned
        m_has_materialized_message = true;
        return m_materialized_message.c_str();
    }
    catch (...) {                                      // <- load-bearing
        return msg;
    }
}
```

The `catch (...)` is not incidental. It is how a `noexcept` function does allocating,
throwing work: on any failure it degrades to the un-decorated message. **Rust has no
mechanism to catch a foreign C++ exception.** Reproducing it would need a C++ shim
containing the try/catch — i.e. adding C++ to the hybrid, which is the opposite of what
a port does. The workspace's `panic = "abort"` closes the remaining door.

Any Rust version would either let the exception propagate out of a `noexcept` function
(`std::terminate`, a behaviour change on the error path) or be unable to detect the
failure at all.

Alongside it: a pure-virtual `message()` dispatch, `std::stringstream` construction,
`ss.str()` returning `std::string` by value, and assignment into a `mutable std::string`
member of `ExceptionWithBacktraceBase`.

### `Backtrace::print(std::ostream&)`

Streams `const char*` into an `std::ostream` via `__put_character_sequence`, which needs
`ostream::sentry` construction/destruction. Feasible in isolation; not worth attempting
given the above.

## Step 5 of the screen is blind here, and this is the fifth such case

Step 5 says: *`grep -c throw`. Non-zero → park.* On this unit it returns **zero**, and
the object nevertheless imports the full exception runtime:

```
nm -u .../util/backtrace.cpp.o
___cxa_allocate_exception   ___cxa_throw
___cxa_begin_catch          ___cxa_end_catch
```

The dependency comes from `catch`, not `throw`, and from libc++ internals inlined in
(`__throw_length_error` is *defined* in this object as a weak symbol). So the grep looks
for the wrong keyword and the `nm -u` symbols that would have caught it are the same ones
`unit-screening.md` already tells you to *discount* — its table says `__cxa_begin_catch`
+ `__gxx_personality_v0` mean only "a `noexcept` landing pad", which was true for
`array_unsigned` and is false here.

Both halves need fixing:

| | current | should be |
|---|---|---|
| grep | `grep -c throw` | `grep -cE '\bthrow\b|\bcatch\b|\btry\b'` |
| `nm -u` reading | `__cxa_begin_catch` ⇒ landing pad, ignore | `__cxa_begin_catch` **plus** a `catch` in the source ⇒ real EH, park. `__cxa_allocate_exception`/`__cxa_throw` are never landing-pad-only |

## What would unblock it

Nothing short of a C++ exception shim — a mechanism for Rust to run a fallible C++
operation and observe failure without unwinding through Rust. That is a larger and less
attractive piece of infrastructure than the vtable/RTTI or ref-translation shims, because
unlike those it has no way to be a thin ABI layer: it needs real C++ in the hybrid.

A better answer for this unit specifically may be that it should not be ported at all.
`Backtrace` exists to decorate exception messages; it produces no `.realm` bytes, and
`evidence-and-linkage.md` classifies it as reachable-but-byte-invisible. The cost is high
and the byte-identity benefit is zero.
