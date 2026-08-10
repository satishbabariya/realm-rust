# `upstream/src/realm/array_key.cpp` — parked 2026-08-10

**A fifth observability category: reachable, but empty.**

This unit passes every screen step perfectly. It is the cleanest row in the entire pending
set:

| step | result |
|---|---|
| 1 reachability | **2 of 2** realm-owned symbols linked |
| 2 vtables | 0 `ZT*` |
| 3 undefined | **zero** undefined symbols of any kind |
| 5 exceptions | 0 `throw|catch|try` |
| 8 VTT | 0 |
| inbound | 13 — among the highest left |

104 lines, two exported symbols, no dependencies at all. On the queue it looks like the
best remaining unit by a wide margin.

## Why it is worthless to port

The entire file is inside `#ifdef REALM_DEBUG`, and this build does not define it. Both
functions compile to nothing:

```
$ llvm-objdump -d array_key.cpp.o
0000000000000000 <__ZNK5realm12ArrayKeyBaseILi0EE6verifyEv>:
       0: pushq %rbp
       1: movq  %rsp, %rbp
       4: popq  %rbp
       5: retq
0000000000000010 <__ZNK5realm12ArrayKeyBaseILi1EE6verifyEv>:
      10: pushq %rbp
      ...
      15: retq

$ otool -l array_key.cpp.o | grep -A4 'sectname __text'
      size 0x0000000000000016        <- 22 bytes for the whole translation unit
```

`ArrayKeyBase<0>::verify()` and `ArrayKeyBase<1>::verify()` are link-list and single-link
backlink consistency checks. They are real code in a debug build and **absent** here. The
zero-undefined-symbols result, which reads as "beautifully self-contained", is the tell:
a function that walks parents, does `dynamic_cast`, and calls `Table::get_opposite_table`
cannot have zero undefined symbols unless it was compiled out.

Porting it means writing two empty Rust functions. They would be byte-identical to the C++
by construction, `make verify` would pass, and the ported-unit count would go up by one
having substantiated nothing. That is the thing `evidence-and-linkage.md` exists to
prevent, arriving through a door the `linked` column cannot see.

## The category, and how to detect it

`linked` measures whether a unit's **symbols survive**, not whether they **do anything**.
A one-line sweep separates the two:

```
otool -l <unit>.cpp.o | grep -A4 'sectname __text' | grep size    # text bytes
```

divided by the `.cpp` line count. Run over all 52 pending units:

| bytes/line | text | lines | unit |
|---|---|---|---|
| **0.21** | **22** | 104 | **array_key** |
| 2.64 | 668 | 253 | `impl/simulated_failure` |
| 4.49 | 687 | 153 | `alloc` |
| 8.43 | 5991 | 711 | `util/interprocess_condvar` |

`array_key` is an order of magnitude below the next entry, so this is an outlier rather
than a widespread pattern — the sweep was worth running precisely to establish that. The
two next-lowest are worth a glance before porting (`impl/simulated_failure` is a test hook
and `alloc.cpp` is mostly inline in its header), but neither is empty.

## If someone wants the TU count down anyway

It is ten minutes: two `#[no_mangle] extern "C"` functions with empty bodies, exact mangled
names `__ZNK5realm12ArrayKeyBaseILi0EE6verifyEv` and `...ILi1EE...`. It is not wrong, it is
just not evidence, and it would make `rust units ported` a slightly less honest number.
Recorded here so the decision is deliberate rather than accidental.
