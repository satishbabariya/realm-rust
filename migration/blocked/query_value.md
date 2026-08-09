# Parked: `upstream/src/realm/query_value.cpp`

223 lines, 16 realm symbols, all linked, live payload, no vtable to synthesize, no VTT.
Parked 2026-08-09 on **step 5** alone — a clean single-criterion exception park.

## Four throws, all in `TypeOfValue`'s constructors

```
 86:  throw query_parser::InvalidQueryArgError(util::format(...));
105:  throw query_parser::InvalidQueryArgError(...);
132:  throw query_parser::InvalidQueryArgError(...);
143:  throw query_parser::InvalidQueryArgError(...);
```

The owning-function test puts every EH site inside a `TypeOfValue` constructor:

```
realm::TypeOfValue::TypeOfValue(long long)
realm::TypeOfValue::TypeOfValue(std::string_view)
(+ their .cold.1 halves)
```

`TypeOfValue` has five constructors and all five are exported and linked. The throwing
ones are the parsing entry points — given an unrecognised type name or an out-of-range
tag, they reject it — so there is no throw-free subset of this unit.

## What throwing it needs

The object leaves this undefined, which is the shopping list:

```
realm::query_parser::InvalidQueryArgError::~InvalidQueryArgError()
realm::InvalidArgument::InvalidArgument(ErrorCodes::Error, std::string_view)
realm::util::format(const char*, std::initializer_list<Printable>)
realm::util::Printable::Printable(StringData)
typeinfo for realm::query_parser::InvalidQueryArgError
```

So Rust would need `__cxa_allocate_exception`, a `realm::InvalidArgument` base
constructor (itself realm code building a `Status`), the derived class's typeinfo, and
`util::format` — which returns `std::string` by value and lives in the parked
`util/to_string.cpp`. That last dependency is the sharp one: **the message is built by a
unit that is itself parked on virtual-base construction**, so this unit is transitively
blocked on two different shims.

## Ranking among exception-shim customers

| unit | exception type | extra needs |
|---|---|---|
| `table_ref` #20 | `InvalidTableRef` over `LogicError` | none |
| **`query_value` #30** | `InvalidQueryArgError` over `InvalidArgument` | `util::format` (parked unit) |
| `uuid` #24 | `InvalidUUIDString` | also a sole-definer vtable |
| `util/time` #16, `util/demangle` #18 | `ExceptionWithBacktrace<T>` | `Backtrace::capture` (parked unit) |

`table_ref` remains the cleanest first customer. This one is second, and only becomes
attractive once `util/to_string` is resolved.
