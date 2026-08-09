# Parked: `upstream/src/realm/column_binary.cpp`

46 lines, one live function, four undefined symbols, no vtable, no `throw`, no STL
return. It passes every step of `.claude/rules/unit-screening.md` and it is not
portable in isolation. Parked 2026-08-09.

## The body

```cpp
BinaryData BinaryColumn::get_at(size_t ndx, size_t& pos) const noexcept
{
    REALM_ASSERT_3(ndx, <, size());
    if (m_cached_leaf_begin <= ndx && ndx < m_cached_leaf_end) {
        return m_leaf_cache.get_at(ndx - m_cached_leaf_begin, pos);
    }
    else {
        BinaryData value;
        auto func = [&value, &pos](BPlusTreeNode* node, size_t ndx_in_leaf) {
            LeafNode* leaf = static_cast<LeafNode*>(node);
            value = leaf->get_at(ndx_in_leaf, pos);
        };
        m_root->bptree_access(ndx, func);
        return value;
    }
}
```

The return type is fine — `BinaryData` is `{const char*, size_t}`, a 16-byte POD. The
problem is everything else:

- `m_root->bptree_access(ndx, func)` is a **C++ function template instantiated on a
  lambda type**. Rust cannot instantiate a C++ template, and cannot hand a closure to
  one. Calling it would mean emitting a C++ thunk, which is C++ this port would be
  adding rather than removing.
- `m_leaf_cache.get_at(...)` and `LeafNode::get_at(...)` are inline members of
  `ArrayBigBlobs`, resolved at compile time into this function.
- `size()`, `m_cached_leaf_begin`, `m_cached_leaf_end` reach into `BPlusTree`'s inline
  accessors and member layout.

Porting this one function means reimplementing B+-tree leaf traversal and blob leaf
access in Rust — i.e. porting `bplustree.cpp`, `array_blobs_big.cpp` and their bases
first. It is a leaf in the *include* graph and an interior node in the graph that
actually matters.

## The screening lesson: a low `nm -u` count can mean the opposite of what it looks like

The screen reads `nm -u <object>` as "how much does this unit depend on", and treats a
short list as good — few external dependencies, so a self-contained port. That reading
is wrong in exactly this case.

`column_binary.cpp.o` has **four** undefined symbols, fewer than any other candidate.
Not because the function is self-contained, but because **everything it calls was
inlined into it**. The dependencies are still there; they have been compiled in rather
than left for the linker, so they do not appear in `nm -u` at all.

So the metric has two opposite meanings and cannot be read alone:

| `nm -u` | possible meaning | portable? |
|---|---|---|
| short | genuinely self-contained (`interprocess_mutex`: 4 libdispatch calls) | yes |
| short | everything inlined from templates and header members | **no** |
| long | calls a lot of out-of-line library code (`utilities`, `fifo_helper`) | usually yes |

The disambiguating check is to look at the **source**, not the object: does the body call
templates, lambdas passed to templates, or inline members of other realm classes? If it
does, a short undefined list is evidence of inlining, not of independence.

`nm -u` is still the right way to choose the *differential shape* — that use is about
what the driver has to link, and is unaffected. It is its use as a portability signal
that needs the source check alongside it.

## What I would need to continue

`bplustree.cpp` and `array_blobs_big.cpp` ported first — both of which own vtables and
are therefore behind the vtable/RTTI shim. This unit is downstream of the same decision
as the other 78.

## Note on the two remaining "clean" units

`array_key.cpp` and `array_unsigned.cpp` were classified clean by the same screen and
should be re-checked against the source before either is assumed portable.
`array_key.cpp`'s live symbols are two `ArrayKeyBase<N>::verify()` instantiations, which
are near-empty in a release build and would be a near-worthless port regardless.
`array_unsigned.cpp` remains the interesting one — its `set_width`/`create`/`insert`/
`erase`/`truncate` are element-width logic, the thing byte-identity exists to check —
but whether it is genuinely self-contained or merely inlines `Array`'s header machinery
is now an open question, and the answer decides whether it is portable.

Did not touch a trace, the comparator, the Makefile, or `upstream/`.
