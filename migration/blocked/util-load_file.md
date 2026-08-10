# `upstream/src/realm/util/load_file.cpp` — parked 2026-08-10

22 lines, which is the whole trap. Two independent blockers.

**Step 1.** 4 of 5 realm-owned symbols linked — and the missing one is the payload. The
survivors are `util::File` scaffolding pulled in by the header. `load_file` itself does not
reach `build/oracle/trace_runner`; nothing in a `REALM_ENABLE_SYNC=OFF` build loads a file
this way.

**Step 5.** The owning-function test names
`realm::util::load_file(std::string const&)` as a throw site — a `realm::` function, not a
libc++ helper.

Third confirmation of the addendum in `evidence-and-linkage.md`: **a partial `linked` count
is not reassurance; ask which side of the split the unit's own payload fell on.** At 22
lines this is the most attractive-looking unit in the queue by size and one of the least
worth porting, which is the queue header's point about size measuring effort rather than
value.
