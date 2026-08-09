# Parked: `upstream/src/realm/impl/output_stream.cpp`

81 lines, three functions, all 8 realm symbols linked and the payload live. Parked
2026-08-09 on **step 2 (definer count 1)** and **step 5 (real exceptions)**.

Notable as the **first unit where reflection #5's definer-count rule produces a park
rather than releasing one**. It released `util/time` and `util/demangle`, whose `ZT*`
had 5 definers; here the same rule holds the line at 1.

## Step 2 — sole definer of one instantiation's vtable

The unit throws `util::overflow_error`, which is
`ExceptionWithBacktrace<std::overflow_error>`, and it is the only translation unit in
`librealm.a` that instantiates that specialisation:

| symbol | definers |
|---|---|
| `vtable` / `typeinfo` / `typeinfo name` for `ExceptionWithBacktrace<std::overflow_error>` | **1** |
| `vtable` / `typeinfo` / `typeinfo name` for `detail::ExceptionWithBacktraceBase` | 5 |

The split within one class family is the whole point of the rule. The **base** is used
everywhere and coalesces, so nothing is orphaned by removing this TU. The
**`overflow_error` instantiation** exists here and nowhere else, so removing this TU
deletes its vtable and RTTI outright, and a Rust port would have to synthesize all
three.

Pre-reflection-#5 the screen would have reported "6 `ZT*` defined" for this unit and for
`util/time`, and parked both for the same stated reason. Only one of those was right.

## Step 5 — two realm-owned throws

```cpp
void OutputStream::write(const char* data, size_t size) {
    do_write(data, size);
    if (int_add_with_overflow_detect(m_next_ref, size))
        throw util::overflow_error("Stream size overflow");
}

ref_type OutputStream::write_array(const char* data, size_t size, uint32_t checksum) {
    ...
    if (int_add_with_overflow_detect(m_next_ref, size))
        throw util::overflow_error("Stream size overflow");
    return ref;
}
```

Owning-function test:

```
realm::_impl::OutputStream::write(char const*, unsigned long)
realm::_impl::OutputStream::write_array(char const*, unsigned long, unsigned int)
(+ their .cold.1 halves)
```

Both realm functions. Two of the three exports throw, and the third (`do_write`) is
what they call, so there is no throw-free subset.

Throwing it from Rust needs `__cxa_allocate_exception`, an exception object carrying a
`Backtrace` (so `Backtrace::capture()`, from the parked `util/backtrace.cpp`), and the
vtable/RTTI from step 2 — which this unit is the sole definer of, so the two blockers
are the same work.

## Also worth noting

`do_write` and `write_array` call `m_out.write(...)` on a `std::ostream&`, so a port
would additionally need `std::ostream::write` — a different iostreams entry point from
the `__put_character_sequence` used by `error_codes` and `status`, and one with no
convenient inline-template form to bind to.

`write_array` also does `reinterpret_cast<const char*>(&checksum)` and writes 4 bytes of
a `uint32_t` — a **little-endian, byte-visible** write into the stream, the same
`memcpy`-of-an-int pattern documented in `object_id`'s entry. If this unit is ever
ported, that is the format decision to mirror rather than tidy.

## What would unblock it

The vtable/RTTI synthesis shim *and* the exception-construction shim, together. Unlike
`util/time` and `util/demangle`, this unit's payload is live and its output reaches the
file, so it is a genuinely worthwhile customer for both — better than either of those
two, and comparable to `table_ref`.
