# `upstream/src/realm/node.cpp` — parked 2026-08-10, pending one decision

**This is the most format-critical unit in the tree and it is blocked on a single
`Cargo.toml` line, not on the RTTI shim.**

## Why it was the next unit

Surfaced by measuring *strong sole-definer* symbols rather than `nm -g`:

| | |
|---|---|
| strong symbols | **8** — `Node::create_node`, `Node::alloc`, `Node::do_copy_on_write`, `Node::calc_byte_len`, `Node::calc_item_count`, `ArrayPayload::~ArrayPayload` ×3 variants |
| lines | 170 |
| `ZT*` | 0 |
| VTT | 0 |
| `throw|catch|try` in source | **0** |
| observability | **byte-visible and TRACED** — every trace allocates nodes |

That symbol list is where element width, header layout and capacity growth are decided —
`format-fidelity.md` opens on exactly this. `array_unsigned.rs` and `array_blob.rs` already
*bind* `Node::alloc`; porting it turns two imports into definitions. It would be only the
second byte-visible-and-traced unit after `array_unsigned`, and by far the most central.

## What stops it

`node.cpp` contains no `throw`, but three of its functions **own throw sites**:

```
llvm-objdump -d -r node.cpp.o | awk '/^[0-9a-f]+ </{fn=$0} /___cxa_throw/{print fn}' | sort -u
  realm::Node::create_node(...)
  realm::Node::alloc(...)
  realm::Node::do_copy_on_write(...)
```

They come from `alloc.hpp:493-510`, where `Allocator::alloc` and `Allocator::realloc_` are
**inline** and throw on a read-only allocator:

```cpp
inline MemRef Allocator::alloc(size_t size)
{
    if (m_is_read_only)
        throw realm::LogicError(ErrorCodes::WrongTransactionState,
                                "Trying to modify database while in read transaction");
    return do_alloc(size);
}
```

Live and reachable, not debug-gated. There is **no out-of-line copy** to bind — measured:
the only allocator symbols in `librealm.a` are the virtual `do_alloc`/`do_realloc`
implementations. Rust could call `do_alloc` through the vtable and skip the check, but that
silently turns "throw on write-in-read-transaction" into "write anyway", which is a real
behaviour change on an error path.

## The finding: throwing is cheap, and the RTTI shim is not the blocker

`exceptions.cpp` **stays C++**. So `LogicError`'s typeinfo, vtable, constructor and
destructor already exist in the link — all four measured present in
`build/oracle/trace_runner`:

```
T __ZN5realm10LogicErrorC1ENS_10ErrorCodes5ErrorENSt3__117basic_string_viewIcNS3_...EE
T __ZN5realm10LogicErrorD1Ev
S __ZTIN5realm10LogicErrorE
```

Rust does not have to *synthesize* any of it. It has to allocate an exception, call the
bound constructor, and call `__cxa_throw` with the bound typeinfo — about twenty lines.

`migration/checks/throw_probe/` does exactly that and settles it empirically. The same Rust
source, built twice:

```
  panic=abort  exit=134  thread caused non-unwinding panic. aborting.
  panic=unwind exit=0    caught LogicError: code=1015
                         what=Trying to modify database while in read transaction
```

**Under `panic = "unwind"`, Rust throws a `realm::LogicError` and C++ catches it by
reference with the right code and message.** Under `panic = "abort"` the unwinder hits the
Rust frame and aborts.

So the blocker is one line in `Cargo.toml`:

```toml
[profile.release]
# Unwinding across the FFI boundary into C++ is undefined behaviour.
panic = "abort"
```

That comment was written before `extern "C-unwind"`, which exists precisely to make this
defined. The probe uses `extern "C-unwind"` throughout.

## What the decision costs

Switching to `panic = "unwind"` is not free and is **not this loop's call**:

- **For:** it unblocks the format core. `node`, `util/file_mapper` (the memory-mapping
  path, where `.realm` bytes reach disk), `util/fifo_helper`, `util/thread` and others are
  parked *only* on throwing. It does not touch the oracle, which contains no Rust, and it
  cannot change `.realm` bytes — it changes Rust-side codegen only.
- **Against:** with `panic = "abort"`, a genuine Rust bug — an `overflow-checks` trap, an
  index panic — aborts loudly at the point of failure. With `panic = "unwind"` that same
  panic unwinds into C++, where a `catch (...)` somewhere up the stack can **swallow it**
  and continue with corrupt state. Given this project's whole premise is that silent
  corruption is the enemy, that is a real cost, not a formality.

A middle path worth considering: `panic = "unwind"` **plus** a `catch_unwind` barrier at
every `#[no_mangle]` entry point that is not deliberately throwing, converting a stray Rust
panic back into an abort. That keeps the loud-failure property for bugs while allowing
deliberate C++ exceptions through.

## Unchanged by this

Units that **catch** are still unportable as whole units — Rust has no `catch`, and that is
independent of the panic mode. `util/backtrace` and `global_key` stay parked for that
reason. The throw/catch split recorded in `migration/blocked/exceptions.md` holds; only the
throw half turns out to be cheap.
