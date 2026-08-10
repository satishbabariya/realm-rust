# `upstream/src/realm/util/terminate.cpp` — parked 2026-08-10

Queue position at parking: #1. It got there by having the cleanest `nm` screen left in
the tree, and the screen was measuring the wrong things.

## What the cheap screen said

| step | result |
|---|---|
| 1 reachability | **2 of 2** realm-owned symbols linked, and they are the payload — `terminate` and `terminate_with_info` |
| 2 vtables | **0** `ZT*` defined |
| 5 exceptions | **0** `throw|catch|try` in the source, and the owning-function test finds only libc++ helpers |
| 8 VTT | 0 — **and this was a lie**, see below |
| inbound | 42 of 67 `Storage` objects reference `util::terminate` |

Everything a `nm`-based screen can see says port it.

## What actually stops it

### 1. It builds a `std::stringstream` inline

`terminate_with_info` opens with `std::stringstream ss;` and streams into it. That single
declaration drags the entire libc++ iostream construction path into this TU. From
`nm -u`:

```
VTT for std::__1::basic_stringstream<char, ...>
vtable for std::__1::basic_streambuf<char, ...>
vtable for std::__1::basic_stringbuf<char, ...>
vtable for std::__1::basic_stringstream<char, ...>
std::__1::ios_base::init(void*)
std::__1::locale::locale() / ~locale()
std::__1::locale::use_facet(std::__1::locale::id&) const
std::__1::ctype<char>::id
std::__1::basic_ios<char, ...>::~basic_ios()
std::__1::basic_ostream<char, ...>::sentry::sentry(...) / ~sentry()
std::__1::ios_base::__set_badbit_and_consider_rethrow()
```

`basic_stringstream` has **virtual bases**, which is what the VTT is for. The constructor
is a template instantiation expanded into this object rather than a symbol to bind, so
Rust would have to reproduce virtual-base offset initialisation for a libc++ type whose
layout is not part of any stable ABI contract this project can rely on.

### 2. `os_log_error` is compiler-generated metadata, not a call

On Apple the termination callback is `nslog`, which calls

```c
os_log_error(OS_LOG_DEFAULT, "%{public}s", message);
```

That macro emits a **format descriptor into a custom section** and calls
`__os_log_error_impl` with a pointer to it. The object confirms both halves:

```
$ otool -l util/terminate.cpp.o | grep sectname
  sectname __oslogstring   segname __TEXT      <- the generated descriptor
$ nm -u util/terminate.cpp.o | grep os_log
  __os_log_default
  __os_log_error_impl
  _os_log_type_enabled
```

Rust cannot emit `__TEXT,__oslogstring` content, and hand-assembling the descriptor means
hand-encoding a private, undocumented format. Getting it wrong changes what the unified
logging system records — a behaviour change **no differential in this repo could see**,
because `os_log` output does not come back in-process to be compared.

`nslog` also `dlsym`s `CLSLog` and, if Crashlytics is loaded, builds a `CFStringRef` and
calls through a function pointer. That part is reproducible; the `os_log` part is not.

## The step-8 check was broken, and this unit is what exposed it

`unit-screening.md` step 8 carried:

```
nm -u $OBJ | grep '^VTT for'
```

**`nm -u` prints mangled names.** That grep matched demangled text against mangled output
and returned 0 for every object ever screened. The rule's claim that it was "validated
across 15 units, zero false positives" was vacuous — it never fired once. The working
form is `grep '^__ZTT'`.

Re-run corrected across all 67 `Storage` objects, **18 units reference a VTT** and none of
them is ported, so the dead check never caused a wrong port. It cost nothing, which is
precisely why it survived fifteen screens. The rule is corrected in place.

## Value, for whoever revisits this

Low, and worth saying explicitly so nobody re-attempts it on the strength of the inbound
count. The unit exports two symbols, is **byte-invisible** (its entire effect is a
diagnostic message and `abort()`), and runs only on a crash path that no trace and no
corpus file reaches. Its 42 inbound references are all `REALM_ASSERT_RELEASE` and
`REALM_UNREACHABLE` sites that never fire in a passing run.

So porting it would buy: no byte-identity coverage, no gate signal, and a hand-encoded
Apple logging descriptor that nothing can verify. Leaving `terminate.cpp` as C++
permanently is the better answer, in the same category as the exception hierarchy
(`migration/blocked/exceptions.md`) — a place where the port deliberately stops.

## What would change the verdict

Nothing short of a decision that the Rust port may print a *different* crash message from
the C++ one. That is not a porting decision; it is a product decision about diagnostics,
and it belongs to a human. If it were taken, the unit becomes easy: two exported
functions, a `Backtrace` bound through three symbols, and a `write(2)` to stderr.
