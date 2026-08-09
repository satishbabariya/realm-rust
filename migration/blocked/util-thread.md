# `upstream/src/realm/util/thread.cpp` — parked 2026-08-09

Queue position at parking: #2 — depth 2, inbound 9, 318 lines, 22 of 22 realm-owned
symbols linked. Live, well-connected, and gated on the exception shim.

## What was measured

`grep -cE '\b(throw|catch|try)\b'` = 15. Per step 5 that is not the verdict on its own,
so the owning-function test was run:

```
llvm-objdump -d -r util/thread.cpp.o \
  | awk '/^[0-9a-f]+ </{fn=$0} /___cxa_(throw|begin_catch|allocate_exception)/{print fn}' | sort -u
```

Five **`realm::`-owned** throw sites, all of them error paths of primitives the rest of
the library depends on:

- `util::Mutex::init_failed(int)`
- `util::Mutex::attr_init_failed(int)`
- `util::CondVar::init_failed(int)`
- `util::CondVar::attr_init_failed(int)`
- `util::Thread::set_name(std::string const&)` (+ a cold path)

These are exactly the symbols that `utilities.cpp`'s file-static `util::Mutex` drags in,
documented in `evidence-and-linkage.md` under differential shapes. They are real
`std::system_error`/`RuntimeError` throws on `pthread_*` failure, not landing pads.

Also 6 `ZT*` defined; not separately resolved, because step 5 already decides it.

## Note on the un-ported half

`util/interprocess_mutex.cpp` is already ported and works, which might suggest the
threading primitives are within reach. They are not the same unit: `InterprocessMutex`
on Darwin is a thin `SemaphoreMutex` wrapper with four undefined symbols, all libdispatch,
and no failure path that throws. `thread.cpp` is where the throwing lives.

Revisit with the exception shim (`migration/blocked/exceptions.md`). This unit only
throws and never catches, so a shim would unblock it.
