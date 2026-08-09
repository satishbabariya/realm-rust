# Parked: `upstream/src/realm/array_blobs_big.cpp`

205 lines, 8 strong symbols, 19/19 realm symbols linked, **traced** (10 hits on
`ArrayBigBlobs::add` across the five traces), no sole-definer vtable, no throws, no VTT.
Parked 2026-08-09 on **one function out of eight**.

This is the first park where the unit is almost entirely portable and the
all-or-nothing rule is what stops it. Worth being precise about, because "7 of 8 done"
is not a reason to park and the eighth one is.

## Seven of the eight are straightforward

`get_at`, `add`, `set`, `insert`, `find_first`, `count` and `verify` need only machinery
this crate already has from `array_blob` and `array_blobs_small`:

- constructing a local `ArrayBlob` (112 bytes, two vptrs at `&vtable[2]` / `&vtable[10]`)
- `set_parent(this, ndx)` with the +56 `ArrayParent` adjustment
- `ArrayBlob::replace` / `get_at` / `blob_replace` — already Rust
- `Array::add`, `insert`, `set`, `set_as_ref`, `init_from_mem`, `blob_size`,
  `destroy_deep(ref, alloc)` — all out-of-line and bindable (12 definitions of the last)
- `get_context_flag_from_header`, `get_size_from_header` — header bits

The layout is the simplest of the three blob units: `sizeof = 112`, an `Array` plus a
single `bool m_nullable` at offset 108, tucked into the base's tail padding. No embedded
sub-arrays, unlike `ArraySmallBlobs`'s 448 bytes.

`verify()` is empty in this build (`#ifdef REALM_DEBUG`).

## The eighth needs B+-tree insertion

```cpp
void ArrayBigBlobs::find_all(IntegerColumn& result, BinaryData value, bool is_string,
                             size_t add_offset, size_t begin, size_t end)
{
    size_t begin_2 = begin;
    for (;;) {
        size_t ndx = find_first(value, is_string, begin_2, end);
        if (ndx == not_found) break;
        result.add(add_offset + ndx);   // <- this
        begin_2 = ndx + 1;
    }
}
```

`IntegerColumn` is `BPlusTree<int64_t>`, and `add` is inline. It expands to
`bptree_insert(ndx, FunctionRef<size_t(BPlusTreeNode*, size_t)>)`, which is why the
object leaves that symbol undefined.

**Half of that is reachable and half is not:**

| piece | status |
|---|---|
| `BPlusTreeBase::bptree_insert(size_t, FunctionRef<…>)` | **bindable** — 1 definition in `librealm.a` |
| `util::FunctionRef` | **constructible** — `{void* m_obj, callback m_callback}`, a concrete type-erased struct taken *by value*, not a template parameter |
| the callback body — the leaf insert | **not reachable** |

The callback receives a `BPlusTreeNode*` and must perform the leaf insertion:
`BPlusTree<int64_t>`'s leaf handling, including widening and node splitting. There is no
out-of-line `add` or `insert` for it anywhere:

```
nm librealm.a | grep -v ' U ' | c++filt | grep -E 'BPlusTree<long long|IntegerColumn'
  realm::IntegerColumn::IntegerColumn(realm::Allocator&, unsigned long)
  realm::IntegerColumn::~IntegerColumn()   (x2)
  ... and nothing else
```

Constructors and destructors only. Reimplementing the insert means porting
`BPlusTree<int64_t>` inside a blob unit — shared infrastructure improvised in one place,
which is what `unit-screening.md` exists to prevent, and which `bplustree.cpp` (13
sole-definer `ZT*`) will need on its own terms.

Note the difference from `column_binary`, which was parked for what looks like the same
thing. There, the callable was handed to a C++ **function template**, which Rust cannot
instantiate or call at all. Here the interface is type-erased and callable; what is
missing is the *body* Rust would have to supply. Closer, but the same answer.

## Why 7-of-8 does not help

A translation unit is all-or-nothing under this harness: the hybrid excludes C++ purely
by link order, so defining seven of eight symbols leaves the eighth undefined,
`librealm.a` supplies the C++ object to resolve it, and the other seven become duplicate
symbols — a hard link failure. There is no partial landing.

## What would unblock it

`BPlusTree<int64_t>` insertion available to Rust — either because `bplustree.cpp` is
ported, or because a leaf-insert entry point becomes callable. Given `bplustree.cpp`
itself needs the vtable/RTTI shim, this unit sits behind that work.

Worth revisiting immediately if that changes: seven eighths of it is already written in
this crate's existing helpers, and it is **traced**, so the gate could judge it.
