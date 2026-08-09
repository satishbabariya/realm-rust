# Parked: `upstream/src/realm/table_ref.cpp`

78 lines, ten exported symbols, all ten realm symbols linked. Parked 2026-08-09 on
**step 5**. No step-2 issue (zero `ZT*` defined), no reachability issue — this is a
clean, unambiguous exception park, and the first one in a while that needed no rule
correction to reach.

## The throw is in the function every other function calls

```cpp
void ConstTableRef::check() const
{
    if (m_table == nullptr)
        throw InvalidTableRef("null");
    if (m_table->get_instance_version() != m_instance_version)
        throw InvalidTableRef(m_table->get_state());
}
```

`operator*` and `operator->` on both `ConstTableRef` and `TableRef` are each two lines:
`check(); return …`. So four of the ten exports funnel through the throw site, and there
is no subset of this unit that avoids it.

Owning-function test:

```
llvm-objdump -d -r $OBJ | awk '/^[0-9a-f]+ </{fn=$0} /___cxa_(throw|begin_catch|allocate_exception)/{print fn}' | sort -u
realm::ConstTableRef::check() const
realm::ConstTableRef::check() const (.cold.1)
realm::ConstTableRef::check() const (.cold.2)
```

A realm function owns every site. Not libc++ spill.

## What throwing `InvalidTableRef` needs

The object leaves these undefined, which is the shopping list for the exception shim:

```
realm::LogicError::LogicError(ErrorCodes::Error, std::string_view)
realm::InvalidTableRef::~InvalidTableRef()
typeinfo for realm::InvalidTableRef
vtable  for realm::InvalidTableRef
```

So Rust would have to `__cxa_allocate_exception`, run a `LogicError` base constructor
that itself builds a `Status`, attach `InvalidTableRef`'s vtable and typeinfo, and hand
the unwinder something a `catch (const LogicError&)` up the stack will match. This is
the richest exception case seen so far — `util/time` and `util/demangle` needed
`ExceptionWithBacktrace<T>`, which at least has no realm-side base constructor to run.

## A second reason, recorded but not decisive

`check()` calls `m_table->get_instance_version()` and `m_table->get_state()`, both
members of `Table`. `get_state()` is out-of-line (undefined in this object, so callable),
but `get_instance_version()` was **inlined** — the object defines
`realm::Allocator::get_instance_version()` as its own coalesced copy. Reimplementing it
in Rust means depending on `Table`'s and `Allocator`'s member layout, which is the
inline-base-helper hazard from `array_unsigned`. Tractable, but it would have to be
measured; the exception blocker makes it moot.

## What would unblock it

The C++ exception-construction shim, in its most demanding form: a realm exception whose
base constructor is itself realm code. If that shim is ever built, `table_ref` is a
better first customer than `util/time` or `util/demangle` — all ten of its symbols are
linked, and unlike those two its payload is live.
