# `upstream/src/realm/util/uri.cpp` — parked 2026-08-10

Fails three screen steps independently, which is a first.

**Step 1 — partial linkage, dead payload.** 6 of 35 realm-owned symbols reach
`build/oracle/trace_runner`. The survivors are error/backtrace scaffolding
(`util::format`, `Backtrace::capture`, `Exception::Exception(Status)`,
`ExceptionWithBacktraceBase::materialize_message`); no `Uri` parsing function is among
them. Same shape as `util/demangle` and `util/timestamp_formatter`: the unit the file is
named for is dead. With `REALM_ENABLE_SYNC=OFF` nothing constructs a `Uri`.

**Step 2 — 9 `ZT*` defined, 3 sole-definer.**

**Step 5 — real throws.** `grep -cE '\b(throw|catch|try)\b'` is 12 and the owning-function
test names `Uri::parse`, `Uri::set_scheme`, `set_auth`, `set_path`, `set_query`,
`set_frag`, and `uri_percent_decode` — all `realm::`, with seven `.cold` paths.

**Step 8 — references a VTT** (`std::basic_stringstream`), so it builds a stringstream
inline. See `migration/blocked/util-terminate.md` for why that is a large surface.

Any one of these would park it. The reachability result alone means a green `make verify`
on this unit would substantiate nothing.
