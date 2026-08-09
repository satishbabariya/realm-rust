# Parked: `upstream/src/realm/uuid.cpp`

126 lines, 8 realm symbols, all linked, live payload. Parked 2026-08-09 on **step 2** and
**step 5**, both genuine and independent.

`unit-screening.md` has said since reflection #1 that "`uuid.cpp` (#24) was rejected
here" at step 5. That was true and it was never written down as a park file; this is the
missing entry, with the step-2 half added.

## Step 2 — sole definer of `InvalidUUIDString`'s vtable

| symbol | definers |
|---|---|
| `vtable` / `typeinfo` / `typeinfo name` for `realm::InvalidUUIDString` | **1** |

This TU is the only source in `librealm.a`. Removing it orphans all three, so a Rust
port must synthesize them — the same obligation as `impl/output_stream` and
`array_with_find`.

## Step 5 — one throw, owned by the constructor

```
llvm-objdump -d -r $OBJ | awk '/^[0-9a-f]+ </{fn=$0} /___cxa_(throw|begin_catch|allocate_exception)/{print fn}' | sort -u
realm::UUID::UUID(realm::StringData)
```

`UUID::UUID(StringData)` throws `InvalidUUIDString` when the input is not a well-formed
UUID. Both constructor variants (`C1`/`C2`) are exported and both are linked, so there is
no throw-free subset — the parsing constructor *is* the unit's main entry point.

The exception carries a message built by
`util::format<StringData&>(const char*, StringData&)`, which this object also emits and
which **returns `std::string` by value** — so the exception shim would need `sret`
handling and `operator new` on top of the vtable synthesis.

## Also present

`UUID::to_base64()` calls `util::base64_encode`, which is already Rust. `UUID::to_string()`
and `is_valid_string` are pure formatting and validation and would port easily on their
own; they are not separable from the constructor under the all-or-nothing rule.

## What would unblock it

Both shims together — vtable/RTTI synthesis *and* exception construction, the latter in
its `std::string`-carrying form. That is the same pairing `impl/output_stream` needs, and
a strictly larger job than `array_with_find`'s vtable-only requirement.

Priority note: `array_with_find` #22 remains the better first customer for the vtable
shim (no exceptions at all), and `table_ref` #20 the better first customer for the
exception shim (no vtable to synthesize). `uuid` needs both at once and should follow
them, not lead.
