# `upstream/src/realm/util/timestamp_formatter.cpp` — parked 2026-08-09

**Partial linkage with a dead payload.** Third confirmation of the step-1 addendum in
`.claude/rules/evidence-and-linkage.md`: a partial `linked` count is not reassurance, and
the question to ask is *which side of the split the unit's own payload fell on*.

`migration/queue.md` reported `linked = 4` of 9 defined. Resolving the 4:

| symbol | linked? |
|---|---|
| `util::MemoryOutputStream::~MemoryOutputStream()` (×2 variants) | **linked** |
| `util::MemoryOutputStreambuf::~MemoryOutputStreambuf()` (×2 variants) | **linked** |
| `util::TimestampFormatter::TimestampFormatter(Config)` (×2 variants) | dead |
| `util::TimestampFormatter::format(long, long)` | dead |
| `util::TimestampFormatter::make_format_segments(Config const&)` | dead |
| `void util::put_time<char, char_traits<char>>(ostream&, tm const&, char const*)` | dead |

Every surviving symbol is a `MemoryOutputStream`/`MemoryOutputStreambuf` destructor —
those belong to `util/memory_stream` (itself already parked), get emitted here because the
header is included, and are coalesced by the linker. **Not one `TimestampFormatter`
symbol reaches `build/oracle/trace_runner`.**

So the unit the file is named for is entirely dead. Porting it would produce a green
`make verify` that substantiates nothing — `evidence-and-linkage.md` category 1, arriving
disguised as category 2.

Same shape as `util/demangle` (5 of 6 linked, the missing one its entire payload). The
pattern is now measured three times and is worth stating as a rule: **when a unit's
linked symbols are all destructors or all come from an included header's types, the unit
itself is dead.** Constructors and payload functions are what carry the signal;
destructors of embedded types are noise.

Timestamp formatting is a logging concern. With `REALM_ENABLE_SYNC=OFF` and no logger
configured by `harness/trace_runner.cpp`, nothing calls it.
