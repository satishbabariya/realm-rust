# `upstream/src/realm/util/file_mapper.cpp` — parked 2026-08-10

Step 5. 15 of 15 realm-owned symbols linked and zero `ZT*` — genuinely live code, parked
on ABI cost rather than deadness.

`grep -cE '\b(throw|catch|try)\b'` is 17, and the owning-function test names five
`realm::` functions plus three `.cold` paths:

```
realm::util::mmap(const FileAttributes&, size_t, uint64_t, unique_ptr<EncryptedFileMapping>&)
realm::util::mmap_anon(size_t)
realm::util::mmap_fixed(int, void*, size_t, File::AccessMode, uint64_t)
realm::util::munmap(void*, size_t)
realm::util::msync(int, void*, size_t)
```

These are the `SystemError`/`AccessError` throws every mapping primitive raises when the
syscall fails. Blocked on the exception shim (`migration/blocked/exceptions.md`); throws
only, never catches, so a shim would unblock it.

One thing to weigh before that happens: this unit is on the **memory-mapping path**, which
is where `.realm` bytes actually reach the disk. Of everything currently parked on the
exception shim it has the strongest claim to byte-identity relevance, and the traces
exercise it constantly. It is the one to port first once throwing is possible.
