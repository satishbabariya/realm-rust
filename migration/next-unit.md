# Next unit: `upstream/src/realm/array_backlink.cpp`

Screened clean on **every** step with the corrected commands, 2026-08-10. Not started.

| step | result |
|---|---|
| 1 reachability | **71 of 71** realm-owned symbols linked |
| 2 vtables/RTTI | 11 `ZT*` defined, **0 sole-definer** (7–38 definers each — `Array`, `ArrayParent`, `BPlusTree<int64_t>` and its `LeafNode`, all coalesced) |
| 4 strong sole-definer | **8**, all `ArrayBacklink` methods |
| 5 exceptions | **none at all** — no `throw`/`catch`/`try` in the source, no realm-owned throw site in the object, and no `ScopeExit`/`unique_ptr`/`FunctionRef` lambda that a grep would miss |
| 8 VTT | 0 |
| size | 277 lines, 7,780 bytes of text |

Step 2 is the one that dead-ended `node.cpp` and it is run here. Step 5 is read rather
than counted, which is what `util/fifo_helper` cost.

The eight exports:

```
ArrayBacklink::add(size_t, ObjKey)
ArrayBacklink::erase(size_t)
ArrayBacklink::remove(size_t, ObjKey)
ArrayBacklink::get_backlink(size_t, size_t) const
ArrayBacklink::get_backlink_count(size_t) const
ArrayBacklink::nullify_fwd_links(size_t, CascadeState&)
ArrayBacklink::verify_backlink(size_t, int64_t)
ArrayBacklink::verify() const
```

The other 34 symbols in the object are `weak private external` `BPlusTree<int64_t>`
instantiations that every including TU emits for itself — they neither force extraction
nor need defining. That is the strong-sole-definer reframing doing its job: `nm -g` reports
42 and the obligation is 8.

## What to expect

**Byte-visible but UNTRACED.** Backlinks are stored data, but no trace creates a link — the
trace schema is int/string/double/bool. So `make verify` proves the link is intact and
nothing else regressed, and the evidence for the unit itself has to be a differential in
`migration/checks/`. Same situation as `array_timestamp`, `unicode` and `decimal128`.

The real work is step 8, and it was not completed. From `nm -u`, the eight methods reach:

- `BPlusTreeBase::bptree_insert` / `bptree_erase` / `bptree_access` / `bptree_traverse`,
  each taking a **`util::FunctionRef<...>`** — a type-erased `{void* obj, fn ptr}` pair, so
  Rust can construct one, but the exact layout must be measured before use
- `Obj`, `ClusterTree::try_get`, `Table::get_opposite_table` / `get_opposite_column`,
  `Cluster::get_col_key`, `Obj::nullify_link`
- `Array::set` / `insert` / `erase` / `truncate` / `find_first` / `move`, all bindable

`FunctionRef` construction is the open question, exactly as `ScopeExitFail` was for
`util/file_mapper`. Measure `sizeof(FunctionRef)` and its two fields, and write a probe
that passes a Rust-built `FunctionRef` into `bptree_traverse`, **before** porting anything.
`unit-screening.md` step 8 calls a lambda handed to a C++ template a park (`column_binary`)
— but that was a template *parameter*, not a type-erased pair. This one is probably
constructible; do not assume either way.
