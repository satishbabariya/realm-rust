# `upstream/src/realm/util/file_mapper.cpp` — unblocked, scoped, not yet ported

**Superseded the 2026-08-10 park.** That park said "blocked on the exception shim". The
shim turned out to be a `Cargo.toml` line (`migration/blocked/node.md`,
`migration/checks/throw_probe/`), and re-screening this unit with the corrected commands
finds nothing else in the way. This file is now the scope note for porting it.

## Full screen, all steps, corrected commands

| step | result |
|---|---|
| 1 reachability | **16 of 16** realm-owned symbols linked |
| 2 vtables/RTTI | **0** `ZT*` defined — nothing to synthesize |
| 3 undefined | 17 realm, 44 non-realm; every one resolvable, see below |
| 4 strong sole-definer | **9**, all `realm::util::` free functions |
| 5 exceptions | 17 `throw`, **0 `catch`, 0 `try`** — `__cxa_begin_catch` present only as `noexcept` landing pads |
| 8 VTT | **0** (`grep '^__ZTT'`, the corrected form) |

Step 2 is run and clean this time. `node.cpp` was written in full before its skipped step 2
surfaced 8 sole-definer RTTI records; that is not repeated here.

The nine exports:

```
util::mmap(const FileAttributes&, size_t, uint64_t, unique_ptr<EncryptedFileMapping>&)
util::mmap_anon(size_t)
util::mmap_fixed(int, void*, size_t, File::AccessMode, uint64_t)
util::munmap(void*, size_t)
util::msync(int, void*, size_t)
util::reserve_mapping(void*, const FileAttributes&, uint64_t)
util::round_up_to_page_size(size_t)
util::do_encryption_read_barrier(const void*, size_t, EncryptedFileMapping*, bool)
util::do_encryption_write_barrier(const void*, size_t, EncryptedFileMapping*)
```

## Why it is worth porting

**Byte-visible and traced, on the path where `.realm` bytes actually reach disk.** Every
trace maps and syncs files, so `make diff-test` judges this directly rather than through a
differential. Only `array_unsigned` has been in that category so far.

## The throws: 8 live on Darwin, 4 types, all constructible

Nine of the 17 `throw` statements are inside `#ifdef _WIN32` or the `#else` of
`#ifndef _WIN32`. The live eight:

| line | type |
|---|---|
| 109, 183 | `realm::AddressSpaceExhausted` |
| 113 | `realm::SystemError` |
| 185, 221, 244, 246 | `std::system_error` |
| 201 | `std::runtime_error` |

*(The guard classifier for this was written twice — the first version keyed on the string
`WIN32` and so marked `#ifndef _WIN32` as Windows-only, i.e. exactly backwards, labelling
all 17 dead. It now carries a four-case self-check that runs before it is used. Third
instance of the same class of bug this week; see the note in `unit-screening.md`.)*

Every constructor needed is **out-of-line and bindable** — confirmed by their appearing as
*undefined* in `file_mapper.cpp.o`, which is proof another object supplies them:

```
std::__1::system_error::system_error(int, const error_category&, const std::string&)
std::__1::system_error::system_error(int, const error_category&, const char*)
std::__1::system_error::~system_error()      typeinfo for std::__1::system_error
std::runtime_error::runtime_error(const std::string&)
std::runtime_error::~runtime_error()         typeinfo for std::runtime_error
std::__1::system_category()                  std::__1::generic_category()
realm::RuntimeError::RuntimeError(ErrorCodes::Error, std::string_view)
realm::SystemError::SystemError(int, std::string_view)        <- definers=1, bindable
realm::AddressSpaceExhausted::~AddressSpaceExhausted()
vtable for realm::AddressSpaceExhausted      typeinfo for realm::AddressSpaceExhausted
```

`AddressSpaceExhausted` is the one with a wrinkle: **its constructor is inline**, so there
is no symbol to call. Its vtable and typeinfo are both bindable, and it derives from
`RuntimeError`, whose `(ErrorCodes::Error, string_view)` constructor *is* bindable. So the
construction recipe is:

1. `__cxa_allocate_exception(sizeof(AddressSpaceExhausted))`
2. call the bound `RuntimeError(ErrorCodes::AddressSpaceExhausted, msg)` on it
3. overwrite the vptr with `&__ZTVN5realm21AddressSpaceExhaustedE + 16`
4. `__cxa_throw` with `__ZTIN5realm21AddressSpaceExhaustedE` and the bound destructor

Step 3 is the delicate one and must be verified by a driver that catches
`const AddressSpaceExhausted&` *by its own type*, not as `RuntimeError&` — catching as the
base would pass even with the vptr left wrong.

## Other dependencies, all bindable

`util::format(const char*, initializer_list<Printable>)`, `util::page_size()`,
`EncryptedFileMapping::read_barrier` / `write_barrier` / destructor,
`EncryptedFile::add_mapping`, `Status::ErrorInfo::create`,
`error::make_error_code(basic_system_errors)`, `Printable::str`. Plus `mmap`/`munmap`/
`msync` from libc, and `std::string` `append`/`insert` for the message building.

## Order of work

1. Move `migration/in-progress/exceptions.rs` into the crate and generalise it from
   `LogicError` to the four types above. It is already proven for the realm case.
2. A throw differential per type, catching each **by its own type**, before porting any
   mapping function.
3. Port the nine functions; `round_up_to_page_size` and the two encryption barriers first,
   since they have no throw path and give a fast link check.
4. `make diff-test` — the traces exercise this constantly, so a mistake shows immediately.

## Estimate, honestly

Comparable to `decimal128` — one iteration for the exception plumbing plus its
differential, one for the mapping functions and the gate. Larger than `node.cpp` looked,
and unlike `node.cpp` there is no RTTI to synthesize, so it should not dead-end.
