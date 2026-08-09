# `upstream/src/realm/global_key.cpp` — parked 2026-08-09

**Unportable for the same reason as `util/backtrace`: it catches.**

This one is painful to park. The screen is otherwise the cleanest in the queue:

- 6 of 6 realm-owned symbols linked
- **0** `ZT*` defined — no vtable or RTTI to synthesize
- 0 `VTT` — no virtual bases
- only 3 realm-undefined symbols, and one of them is `util::sha1`, **already ported**
- byte-visible: `GlobalKey` is the 128-bit object identity stored in files

## What stops it

`grep -nE '\b(throw|catch|try)\b' global_key.cpp` returns 7 hits, and the shape matters
more than the count:

```
40:    try {
55:    catch (const InvalidArgument&) {
72,79,83,92,97:  throw InvalidArgument(ErrorCodes::InvalidArgument, "Invalid object ID.");
```

The owning-function test confirms both halves are real realm code, not inlined libc++
helpers:

```
llvm-objdump -d -r global_key.cpp.o | awk '/^[0-9a-f]+ </{fn=$0} /___cxa_/{print fn}'
  realm::GlobalKey::from_string(realm::StringData)        + 5 cold paths
  realm::operator>>(std::istream&, realm::GlobalKey&)
```

`from_string` throws; `operator>>` calls it inside `try`/`catch (const InvalidArgument&)`
and converts the exception into a stream failure state. That `catch` is **load-bearing**
— it is how `operator>>` reports a malformed key without throwing at its caller.

**Rust cannot catch a foreign C++ exception**, and `panic = "abort"` closes the other
door. This is the exact shape that makes `util/backtrace` unportable
(`materialize_message` is `noexcept` around a `try`/`catch (...)`).

## Why an exception shim would not rescue it

A shim that supplies `__cxa_throw` plus RTTI records would unblock units that only
*throw*. It does nothing for units that *catch*, which need a personality routine and
landing pads Rust does not emit. `global_key` needs both halves.

The only route is to split the unit: `from_string` and `to_string` in Rust behind a
C++-side wrapper that keeps `operator>>` in C++. That means shipping a unit that is
half-ported, which the one-unit-per-commit rule and the link-order exclusion mechanism
(a TU is all-or-nothing) do not currently support.

This is the second unit parked on `catch` specifically. It confirms that extending
step 5's grep from `throw` to `throw|catch|try` was correct — the old grep would have
cleared this unit and the port would have failed at the link, or worse, linked and
silently changed `operator>>`'s failure behaviour.
