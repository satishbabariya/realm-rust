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

Blocked on the exception shim (`migration/blocked/exceptions.md`). It only throws and
never catches, so a shim would unblock it.
