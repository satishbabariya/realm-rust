# `upstream/src/realm/exceptions.cpp` — parked 2026-08-09

**This unit is not blocked on the vtable/RTTI shim. It *is* the shim.**

Queue position at parking: #1 — depth 2, inbound 11, 180 lines, **128 of 128 realm-owned
symbols linked**. The most reachable unit remaining, and the one with the highest inbound
count in the pending set. It is being parked anyway.

## What was measured

Step 2 of `.claude/rules/unit-screening.md`, done properly — definer count, not presence:

```
nm exceptions.cpp.o | grep -v ' U ' | grep -oE '__ZT[VIS][A-Za-z0-9_]*' | sort -u   # 102
nm -m librealm.a | grep -v '(undefined)' | awk '{print $NF}' | sort | uniq -c       # definer table
```

Definer-count histogram over all 102: **`{1: 102}`**. Every single one is sole-definer.

That is the whole realm exception hierarchy's typeinfo, typeinfo-name and vtable:
`LogicError`, `RuntimeError`, `KeyNotFound`, `NoSuchTable`, `NotNullable`, `OutOfBounds`,
`SystemError`, `StaleAccessor`, `KeyAlreadyUsed`, `InvalidArgument`, `FileAccessError`,
`query_parser::SyntaxError`, `query_parser::InvalidQueryError`, and the rest.

Also step 5: `grep -cE '\b(throw|catch|try)\b'` = 5, and the owners are `realm::`
functions — this is where the exceptions are actually thrown from.

## Why that is a park and not a task

Porting this unit means synthesizing, from Rust, 102 correct Itanium-ABI vtable and RTTI
records — including the inheritance edges between them, since `catch (const LogicError&)`
elsewhere in the tree walks `__class_type_info::__do_upcast` through exactly these
records. Get one base-class pointer wrong and a `catch` clause silently stops matching,
which is a behaviour change no `.realm` byte can show.

This is shared infrastructure, and `unit-screening.md` step 2 says not to improvise it
inside one unit. It is a project, not a unit.

## What it gates

Everything. As of this screen the head of the queue is almost entirely exception-gated:

| unit | linked | why it waits on this |
|---|---|---|
| `util/thread` #2 | 22/22 | five realm-owned throw sites: `Mutex::init_failed`, `Mutex::attr_init_failed`, `CondVar::init_failed`, `CondVar::attr_init_failed`, `Thread::set_name` |
| `tokenizer` #7 | 12/12 | throws from `Tokenizer::get_search_tokens`, **and** owns 6 sole-definer `ZT*` of its own |
| `global_key` | 6/6 | throws `InvalidArgument` from `from_string`, and **catches** it in `operator>>` |
| `util/fifo_helper` #6 | 7/7 | every one of its 7 realm-undefined symbols is exception machinery |
| `util/misc_ext_errors`, `util/basic_system_errors`, `obj_list` | — | already parked on this same shim |

## What would unblock it

An exception/RTTI shim, built once and shared:

1. `__cxa_allocate_exception` / `__cxa_throw` with a correctly constructed exception
   object, and
2. emitted `__ZTI*` / `__ZTS*` / `__ZTV*` records matching the Itanium layout, with the
   `__si_class_type_info` / `__vmi_class_type_info` base edges the hierarchy needs.

The honest alternative, and probably the better one: **leave `exceptions.cpp` as C++
permanently** and treat the exception hierarchy as a boundary the port does not cross.
Rust cannot catch a foreign C++ exception and `panic = "abort"` closes the other door, so
every unit that catches is unportable regardless of what shim exists — `util/backtrace`
and `global_key` are already in that category for that reason alone. A shim would unblock
the units that only *throw*; it would not unblock the ones that *catch*.

That decision is a human's to make. It changes what "port realm-core to Rust" means.
