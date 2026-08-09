# `upstream/src/realm/tokenizer.cpp` — parked 2026-08-09

Parked on **both** step 2 and step 5 — the first unit in this queue to fail two
independent screen steps.

## Step 2: sole-definer vtables

```
nm tokenizer.cpp.o | grep -v ' U ' | grep -oE '__ZT[VIS][A-Za-z0-9_]*' | sort -u
```

Six symbols, and the definer count across `librealm.a` is **1 for every one of them**:

| symbol | definers |
|---|---|
| `__ZTVN5realm9TokenizerE`, `__ZTIN5realm9TokenizerE`, `__ZTSN5realm9TokenizerE` | 1 |
| `__ZTVN5realm16DefaultTokenizerE`, `__ZTIN5realm16DefaultTokenizerE`, `__ZTSN5realm16DefaultTokenizerE` | 1 |

This is the key-function TU for `Tokenizer` and `DefaultTokenizer`. Removing it orphans
both vtables; Rust would have to synthesize them, including `__cxa_pure_virtual` in the
slots of `Tokenizer`'s pure virtuals. Same shim as `exceptions.cpp`, and the same
verdict: not something to improvise inside one unit.

## Step 5: a realm-owned throw

`grep -cE '\b(throw|catch|try)\b'` = 3, and the owning-function test names
`realm::Tokenizer::get_search_tokens()` as the throw site — a `realm::` function, not an
inlined libc++ helper. Real EH.

## Reachability, for the record

12 of 12 realm-owned symbols link, so this is genuinely live code — the park is about
ABI cost, not deadness. Full-text search tokenization is reached through
`index_string.cpp`.

Revisit when the vtable/RTTI shim exists (see `migration/blocked/exceptions.md`). Note
that `tokenizer` only throws and never catches, so unlike `global_key` a shim would
actually unblock it.
