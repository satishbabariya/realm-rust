# Parked: `upstream/src/realm/util/compression.cpp`

947 lines. Parked 2026-08-09 on **two independent criteria**, either of which alone is
sufficient: it is the key-function TU for four externally-visible classes (step 2), and
it throws real C++ exceptions from realm code (step 5).

This is the cleanest park in the queue so far — both criteria are what
`.claude/rules/unit-screening.md` explicitly says to park for, and the screen took about
a minute with the corrected commands.

## Step 2 — key-function TU for seven classes

```
nm $OBJ | grep -v ' U ' | grep -E '__ZT[VIS]'     -> 19 symbols
```

| Class | vtable | typeinfo | visibility |
|---|---|---|---|
| `realm::util::SimpleInputStream` | yes | yes | **external (`S`)** |
| `realm::util::compression::CompressMemoryArena` | yes | yes | **external (`S`)** |
| `realm::util::InputStream` | — (abstract) | yes | **external (`S`)** |
| `realm::util::compression::Alloc` | — (abstract) | yes | **external (`S`)** |
| `(anonymous)::DecompressInputStreamLibCompression` | yes | yes | local (`s`) |
| `(anonymous)::DecompressInputStreamNone` | yes | yes | local (`s`) |
| `(anonymous)::ErrorCategoryImpl` | yes | yes | local (`s`) |

The four external ones are the problem. `SimpleInputStream`'s vtable and
`InputStream`'s typeinfo are **linked into `trace_runner`** and are therefore consumed
by other translation units — porting this file means synthesizing vtables and RTTI that
the rest of the library depends on, not just ones private to the unit. The three
anonymous-namespace classes would additionally each need their own.

`util/basic_system_errors` needed *one* synthesized vtable over a libc++ base. This
needs seven, four of them part of realm's public ABI.

## Step 5 — real exceptions, owned by realm functions

Source has five `throw` statements, all `std::system_error`:

```
228:  throw std::system_error(make_error_code(compression::error::decompress_error), m_strm.msg);
317:  throw std::system_error(compression::error::decompress_error);
350:  throw std::system_error(compression::error::corrupt_input);
367:  throw std::system_error(compression::error::corrupt_input);
898:  throw std::system_error(ec);
```

The owning-function test (step 5, added at reflection #4) agrees — these are not libc++
weak-helper spill:

```
llvm-objdump -d -r $OBJ | awk '/^[0-9a-f]+ </{fn=$0} /___cxa_(throw|begin_catch|allocate_exception)/{print fn}' | sort -u
```

```
realm::util::compression::allocate_and_compress_nonportable(...)
realm::util::compression::decompress_nonportable_input_stream(...)
(anonymous namespace)::DecompressInputStreamLibCompression::next_block()
___clang_call_terminate                              <- landing pad, ignore
std::__1::__throw_length_error[abi:nqe210106](...)   <- libc++ helper, ignore
```

Three `realm::`-owned throw sites. Throwing `std::system_error` from Rust needs
`__cxa_allocate_exception`, a correctly constructed exception object with a
`std::error_code` member, and matching RTTI for the unwinder to match a
`catch (std::system_error&)` anywhere up the stack. Same class of work as step 2, and
this unit needs both at once.

## Reachability, for completeness

12 of 34 defined symbols reach `build/oracle/trace_runner`, and **none of the 12 is a
compression function**. What is linked is the `InputStream`/`SimpleInputStream` vtable
and RTTI, `SimpleInputStream::next_block`, its two destructor variants, and
`Buffer<char>::resize`. Everything named `compress`/`decompress` —
`compression::decompress`, `compress_bound`, `error_category`, `make_error_code`,
`CompressMemoryArena::alloc`/`free` — is **not linked**.

So even setting both shims aside, the part of this unit that a port would be *about*
falls in the "unreachable" category of `evidence-and-linkage.md`: the gate could not
distinguish a correct compression port from an empty one. What survives into the binary
is only the stream-accessor scaffolding.

That is a third, independent reason not to port it, and it suggests the unit is
mis-placed in the queue: `gen_queue.py` ranks by lines and include depth, and 947 lines
of zlib/libcompression wrapper score high on the first while contributing nothing the
harness can see.

## What would unblock it

All three of:

1. The vtable/RTTI synthesis shim, extended to **externally-visible** vtables — a
   larger case than `obj_list` or `util/basic_system_errors`, which only need
   unit-private ones.
2. A C++ exception-construction shim (`std::system_error` with a live `error_code`),
   which is the counterpart of the `catch` shim `util/backtrace` needs.
3. A trace that actually compresses something, or the honest conclusion that this unit
   should stay C++ because nothing the harness measures depends on it.

Item 3 is the one to settle first: if the compression entry points are never linked,
items 1 and 2 buy nothing here.

## Audit 2026-08-09 (during the `util/time` screen)

**The park stands.** The "key-function TU" wording above is loose — four of the seven
`ZT*` symbols are `weak external`, not strong, so they are not key-function emissions in
the strict sense. The conclusion is unaffected, because what matters is the **definer
count**, and this object is the sole definer of every one of them:

```
nm -m librealm.a | grep " <symbol>$" | grep -vc undefined
  vtable   for realm::util::SimpleInputStream                1
  vtable   for realm::util::compression::CompressMemoryArena 1
  typeinfo for realm::util::InputStream                      1
  typeinfo for realm::util::compression::Alloc               1
```

Removing this TU orphans all four, so a port must synthesize them. The three
anonymous-namespace classes are `non-external` and therefore definer-count 1 by
construction.

Contrast `util/time.cpp`, screened the same day: its `ZT*` are also `weak external` but
have **5** definers, so removing that TU orphans nothing and step 2 does not apply to it
at all. See `migration/blocked/util-time.md` for the amendment this produced.

## Audit, reflection #5 (2026-08-09)

Both observations from this park were promoted, one in a sharper form than proposed.

- The step-2 definer counts here (**1** for all four externally-visible `ZT*`, despite
  the `weak external` attribute) are now the worked example of "sole definer ⇒ park" in
  `.claude/rules/unit-screening.md`. Verdict unchanged.
- This entry proposed: *"when step 1 shows partial linkage, check whether the linked
  subset contains the functions the unit is named for."* A second occurrence
  (`version.cpp`, 8/14 naive and **0/6** realm-owned) turned that judgement call into a
  mechanical filter, and it is the filter that was promoted: **intersect realm-owned
  symbols only**, `^__ZN[A-Z]*5realm`. Re-measured here, this unit is **4 of 20** realm
  symbols linked, and the surviving four are the `InputStream`/`SimpleInputStream` vtable
  and RTTI — scaffolding, not payload. The "12 of 34" in the body above was inflated by
  eight libc++ weak helpers that link regardless of this TU.
