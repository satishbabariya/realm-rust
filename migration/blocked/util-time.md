# Parked: `upstream/src/realm/util/time.cpp`

48 lines, two functions. Parked 2026-08-09 on **step 5 (real exceptions)** — and
explicitly *not* on step 2, which is where the screen first sent it and which turns out
not to apply.

## The unit

```cpp
std::tm localtime(std::time_t t) {
    std::tm tm;
    if (!localtime_r(&t, &tm)) throw util::invalid_argument("localtime_r() failed");
    return tm;
}
std::tm gmtime(std::time_t t) { /* same shape */ }
```

Four `throw util::invalid_argument(...)` across the two functions, and the
owning-function test puts every EH site in `realm::util::localtime` / `realm::util::gmtime`:

```
llvm-objdump -d -r $OBJ | awk '/^[0-9a-f]+ </{fn=$0} /___cxa_(throw|begin_catch|allocate_exception)/{print fn}' | sort -u
realm::util::localtime(long)
realm::util::gmtime(long)
```

Not libc++ spill. `util::invalid_argument` is
`ExceptionWithBacktrace<std::invalid_argument>`, so throwing it from Rust needs
`__cxa_allocate_exception`, a constructed exception object carrying a `Backtrace`,
matching RTTI for the unwinder, **and** `Backtrace::capture()` — which lives in
`util/backtrace.cpp`, itself parked. Transitively blocked as well as directly.

## Step 2 does **not** apply here, and that is a correction to the rule

The screen flags 6 defined `ZT*` symbols and calls it a key-function TU:

```
S vtable   for realm::util::detail::ExceptionWithBacktraceBase
S typeinfo for realm::util::detail::ExceptionWithBacktraceBase
S vtable   for realm::util::ExceptionWithBacktrace<std::invalid_argument>
...
```

That is wrong. `unit-screening.md` step 2 says a hit means "the compiler emits the
vtable, typeinfo and typeinfo-name **here and nowhere else**". That holds for a class
with a *key function* — the first non-inline, non-pure virtual. `ExceptionWithBacktraceBase`
has no key function (its virtuals are inline or pure), and
`ExceptionWithBacktrace<std::invalid_argument>` is a template instantiation. Both get
**`weak external`** vtables emitted in *every* TU that needs them, and the linker
coalesces:

```
nm -m .../util/time.cpp.o | grep ExceptionWithBacktraceBase
  weak external                    typeinfo for ...ExceptionWithBacktraceBase
  weak external automatically hidden vtable for ...ExceptionWithBacktraceBase

# objects in librealm.a that DEFINE it:
5
```

Five definers. Removing `util/time.cpp.o` orphans nothing, and Rust would not have to
synthesize anything.

**The distinguishing measurement is not the linkage attribute but the definer count:**

```
nm -m librealm.a | grep " <mangled-ZT-symbol>$" | grep -vc undefined
```

| Definers | Meaning | Step 2 verdict |
|---|---|---|
| 1 | this TU is the sole source; removing it orphans the vtable | **park** — the shim must synthesize it |
| >1 | coalesced weak vtable, other TUs supply it | **not a park** on these grounds |

`non-external` (anonymous-namespace) ZT\* are always definer-count 1 by construction.

### Re-checking the two units already parked on step 2

Both stand, and now for a measured reason rather than a pattern match:

| Unit | ZT\* linkage | definers | verdict |
|---|---|---|---|
| `util/basic_system_errors` | all 3 `non-external` (anon namespace) | 1 | park stands |
| `util/compression` | 3 `non-external` + 4 `weak external` | **1** for each of `SimpleInputStream`, `CompressMemoryArena`, `InputStream`, `Alloc` | park stands |

`util/compression` is the sole definer of all four externally-visible vtables/typeinfos
despite their `weak` attribute — so the park file's "key-function TU" wording is loose
but its conclusion is right. Audit note added there.

## What would unblock it

The C++ exception-construction shim (throwing a `realm::util::ExceptionWithBacktrace<T>`
with correct RTTI), plus `util/backtrace.cpp` un-parked so `Backtrace::capture()` exists
in Rust. Both are the same infrastructure `util/compression` and `uuid` wait on.

Given the unit is two `strftime`-adjacent wrappers whose output never reaches a `.realm`,
it is a poor first customer for that shim. Prefer a unit where the exception path is
incidental rather than the entire body.
