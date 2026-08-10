# `upstream/src/realm/node.cpp` — parked 2026-08-10 (second attempt)

**Written and working, then reverted at the link.** The Rust is in
`migration/in-progress/node.rs` and is not compiled into the crate. The tree is back at
15 units with `make verify` exit 0.

## What was completed

All 8 function symbols, ported and building: `Node::create_node`, `Node::calc_byte_len`,
`Node::calc_item_count`, `Node::alloc`, `Node::do_copy_on_write`, and
`ArrayPayload::~ArrayPayload` in its D0/D1/D2 variants. Everything measured rather than
assumed —`Node` layout by `-fdump-record-layouts`, five vtable slot indices by the Itanium
pointer-to-member encoding, the 8-byte header bit rules transcribed from
`node_header.hpp`, and the `REALM_ASSERT_RELEASE` string lifted out of `node.cpp.o`'s
`__cstring` so the abort message matches byte for byte.

Two of those slots — `Allocator::do_translate` at byte 48 and `ArrayParent::get_child_ref`
at 16 — were measured independently by `array_unsigned.rs` months earlier and agree, which
is the cross-check that the declaration-order model is right.

The throwing half also worked: the `panic = "unwind"` switch plus
`migration/in-progress/exceptions.rs` reproduces `Allocator::alloc`'s inline
`LogicError(WrongTransactionState)` correctly.

## What stopped it

`make hybrid` failed with **8 duplicate symbols**. `node.cpp.o` was extracted from
`librealm.a` anyway, because it is the **sole definer of eight RTTI symbols** that other
translation units reference:

```
definers=1  vtable for realm::Node
definers=1  vtable for realm::ArrayPayload
definers=1  typeinfo for realm::Node            <- referenced
definers=1  typeinfo for realm::ArrayPayload    <- referenced
definers=1  typeinfo for realm::NodeHeader
definers=1  typeinfo name for realm::Node
definers=1  typeinfo name for realm::ArrayPayload
definers=1  typeinfo name for realm::NodeHeader
```

`Node`'s key function is `calc_byte_len` — the first non-inline, non-pure virtual — so
`node.cpp` is the key-function TU for `Node` *and*, through `ArrayPayload::~ArrayPayload`,
for `ArrayPayload`. Removing it orphans all eight.

This is step 2 of `unit-screening.md`, exactly as written, with the definer count exactly
at the park threshold. **I did not run step 2 on this unit.** Steps 1, 4, 5 and 8 were run;
step 2 was skipped because the strong-symbol reframing had just made step 4 feel like the
interesting one. The screen is an ordered list precisely so that this cannot happen.

## The second finding: the realm-symbol filter drops all RTTI

`evidence-and-linkage.md` and step 4 both filter realm-owned symbols with:

```
grep -E '^__ZN[A-Z]*5realm'
```

That matches `__ZN5realm…` and `__ZNK5realm…`. It **does not match `__ZTIN5realm…`,
`__ZTSN5realm…` or `__ZTVN5realm…`**, because after `__Z` the pattern demands `N` and
those have `T`. Demonstrated:

```
$ printf '__ZTIN5realm4NodeE\n__ZN5realm4Node5allocEmm\n' | grep -E '^__ZN[A-Z]*5realm'
__ZN5realm4Node5allocEmm            <- the typeinfo was silently dropped
```

So every "strong sole-definer symbol" count published in `JOURNAL.md` and in step 4's table
**excluded RTTI**. `node`'s obligation is not 8 symbols, it is 16: eight functions and eight
RTTI records. The corrected pattern is `^__ZT?[A-Z]*N?[A-Z]*5realm`, or more simply
`^__Z.*5realm`, filtered against libc++ by requiring `5realm` rather than by shape.

This is the same failure as the step-8 VTT grep two iterations ago: **a pattern that has
never been checked against an input it should match.** Both were "validated" by producing
plausible output on inputs that did not exercise them.

## What would unblock it

The RTTI shim, and this is a better first customer than `exceptions.cpp` (102 records) or
`util/logger` (18): **three classes, eight records, and the inheritance is trivial.**

- `NodeHeader` — empty, no bases, no virtuals → plain `__class_type_info`, 16 bytes
- `ArrayPayload` — no bases, polymorphic → `__class_type_info` + a vtable
- `Node` — single inheritance from `NodeHeader` → `__si_class_type_info`, one base pointer

The risk to respect: `dynamic_cast<Array*>` and `dynamic_cast<Cluster*>` appear elsewhere
in the tree, and a wrong base pointer in `Node`'s `__si_class_type_info` makes those casts
silently return null rather than crash. That is a behaviour change **no `.realm` byte would
show**, which is why this belongs in a shared, separately-tested shim rather than
improvised inside this unit — the position `migration/blocked/exceptions.md` already takes.

## Worth keeping from the attempt

`migration/in-progress/exceptions.rs` is independent of all this and is proven working by
`migration/checks/throw_probe/`. Whichever unit uses throwing first should adopt it as-is.
`util/file_mapper` remains the best candidate: it is on the memory-mapping path where
`.realm` bytes reach disk, the traces exercise it constantly, and it throws without
catching. Its RTTI position has not been checked — **run step 2 on it before starting.**
