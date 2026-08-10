# `upstream/src/realm/util/logger.cpp` — parked 2026-08-10

Step 2. 46 of 46 realm-owned symbols linked, **zero** `throw|catch|try` — and **21 `ZT*`
defined, 18 of them sole-definer** across `librealm.a`.

```
nm util/logger.cpp.o | grep -v ' U ' | grep -oE '__ZT[VIS][A-Za-z0-9_]*' | sort -u   # 21
# definer count via: nm -m librealm.a | grep -v '(undefined)' | awk '{print $NF}' | sort | uniq -c
```

This is the key-function TU for the `Logger` hierarchy — `Logger`, `RootLogger`,
`StderrLogger`, `ThreadSafeLogger`, `PrefixLogger`, `LocalThreadFileLogger` and friends.
Removing it orphans 18 vtable/typeinfo records that Rust would have to synthesize,
including the base edges between them.

Same shim as `migration/blocked/exceptions.md`, same verdict: shared infrastructure, not
something to improvise inside one unit. This unit only *defines* vtables and never
catches, so an RTTI shim would genuinely unblock it.

Worth noting for whoever builds that shim: `logger.cpp` is a better first customer than
`exceptions.cpp`. 18 records against 102, no exception hierarchy semantics riding on the
base edges, and a `catch` clause that stops matching cannot silently corrupt anything here
because nothing catches a `Logger`.
