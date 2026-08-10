# `upstream/src/realm/util/fifo_helper.cpp` — parked 2026-08-10

Step 5. 7 of 7 realm-owned symbols linked, zero `ZT*` — and real, realm-owned throw sites.

`grep -cE '\b(throw|catch|try)\b'` is 4, and the owning-function test names realm
functions rather than inlined libc++ helpers:

```
llvm-objdump -d -r util/fifo_helper.cpp.o \
  | awk '/^[0-9a-f]+ </{fn=$0} /___cxa_(throw|begin_catch|allocate_exception)/{print fn}' | sort -u
  realm::util::create_fifo(std::string_view)
  realm::util::try_create_fifo(std::string_view, bool)
```

Every one of its seven realm-undefined symbols is exception machinery —
`FileAccessError`'s constructor and destructor, `RuntimeError(Status&&)`,
`SystemError::~SystemError`, `Status::ErrorInfo::create`, `util::format`, and
`error::make_error_code`. The unit exists to turn a failed `mkfifo` into a typed
exception; that *is* its behaviour.

## Corrected 2026-08-10: it **catches**, and that is permanent

The original text here said "it only throws and never catches, so a shim would unblock
it." **That was wrong**, and it would have sent the next reader at a unit that cannot be
ported. `try_create_fifo` is:

```cpp
bool try_create_fifo(std::string_view path, bool has_more_fallbacks)
{
    if (has_more_fallbacks) {
        try {
            create_fifo(path);
            return true;
        }
        catch (...) {
            return false;          // <- consumes the exception and returns a value
        }
    }
    ...
}
```

That is **catch-and-continue**, the one category of the three that no setting fixes:
Rust has no `catch`, and unlike throwing (a `Cargo.toml` line) and RAII cleanup (a `Drop`),
there is nothing to switch on. Same shape as `util/backtrace` and `global_key`.

How the error was made is worth recording: the original screen ran
`grep -cE '\b(throw|catch|try)\b'`, got 4, saw that the *owning-function* test named two
`realm::` throw sites, and concluded "throws". It never asked what the other two hits were
— they are the `try` and the `catch (...)` above. **A count is not a classification.** The
corrected step 5 in `unit-screening.md` now asks for the exception-path *behaviour*, and
this unit is why the three-way split has to be applied per function, not per unit:
`create_fifo` throws and would be portable; `try_create_fifo` catches and is not, and they
share a translation unit.

## What would unblock it

Nothing available. The only route is splitting the TU, which the all-or-nothing link rule
does not allow, or accepting that `try_create_fifo` stays C++ — which means the whole unit
stays C++.
