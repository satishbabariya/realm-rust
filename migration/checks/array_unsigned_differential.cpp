// Differential driver for upstream/src/realm/array_unsigned.cpp.
//
// Why this exists even though diff-test covers the unit
// -----------------------------------------------------
// Instrumenting all ten exported functions and running every trace showed that
// `erase_churn` and `many_commits` reach create/insert/erase/get/lower_bound/
// update_from_parent, and nothing reaches **set**, **truncate** or **upper_bound**.
// Those three write array payload and header bytes exactly like the others do, so a
// wrong element-width decision in them would be invisible to `make verify`.
//
// This driver hits all of them, across every width ArrayUnsigned can hold, and dumps
// the raw bytes of the node after each mutation — header included, because the width
// and size fields live there and a port that gets the payload right and the header
// wrong still produces an unreadable file.
//
// Build and run via migration/checks/run_array_unsigned_differential.sh.

#include <realm/array_unsigned.hpp>
#include <realm/alloc.hpp>

#include <cstdint>
#include <cstdio>
#include <vector>

using namespace realm;

namespace {

// Dump the whole node: header bytes (which carry width and size) then the payload.
// Byte-for-byte, because that is the acceptance criterion for the whole project.
void dump(const char* tag, const ArrayUnsigned& a)
{
    MemRef mem = a.get_mem();
    const char* header = mem.get_addr();
    size_t bytes = NodeHeader::get_byte_size_from_header(header);
    std::printf("%-28s width=%zu size=%zu hdr_width=%u hdr_size=%zu bytes=%zu |", tag, a.get_width(), a.size(),
                unsigned(NodeHeader::get_width_from_header(header)), NodeHeader::get_size_from_header(header),
                bytes);
    for (size_t i = 0; i < bytes; ++i)
        std::printf("%02x", static_cast<unsigned char>(header[i]));
    std::printf("\n");
}

void dump_values(const char* tag, const ArrayUnsigned& a)
{
    std::printf("%-28s values=", tag);
    for (size_t i = 0; i < a.size(); ++i)
        std::printf("%llu,", static_cast<unsigned long long>(a.get(i)));
    std::printf("\n");
}

// lower_bound / upper_bound over a spread of probes, including values that exceed
// INT64_MAX — realm::lower_bound<w> compares as int64_t, so those go negative.
void dump_bounds(const char* tag, const ArrayUnsigned& a)
{
    static const uint64_t probes[] = {0,
                                      1,
                                      2,
                                      127,
                                      128,
                                      255,
                                      256,
                                      65535,
                                      65536,
                                      0xFFFFFFFFull,
                                      0x100000000ull,
                                      0x7FFFFFFFFFFFFFFFull,
                                      0x8000000000000000ull,
                                      0xFFFFFFFFFFFFFFFFull};
    std::printf("%-28s lb=", tag);
    for (uint64_t p : probes)
        std::printf("%zu,", a.lower_bound(p));
    std::printf(" ub=");
    for (uint64_t p : probes)
        std::printf("%zu,", a.upper_bound(p));
    std::printf("\n");
}

void probe_all(const char* tag, const ArrayUnsigned& a)
{
    dump(tag, a);
    dump_values(tag, a);
    dump_bounds(tag, a);
}

// One full lifecycle at a given starting ubound, so create() picks each width.
void exercise(uint64_t initial_ubound)
{
    std::printf("=== exercise ubound=%llu ===\n", static_cast<unsigned long long>(initial_ubound));
    Allocator& alloc = Allocator::get_default();
    ArrayUnsigned a(alloc);
    a.create(0, initial_ubound);
    probe_all("created", a);

    // Ascending inserts at the end. Values chosen to sit just under and just over
    // each width boundary so the widening decision is exercised from every side.
    static const uint64_t vals[] = {0, 1, 2, 3, 5, 8, 13, 21, 34, 55, 89, 144, 233, 254, 255};
    for (uint64_t v : vals)
        a.add(v);
    probe_all("after ascending adds", a);

    // set() -- UNTRACED. Includes values that force widening at each boundary.
    a.set(0, 7);
    probe_all("set[0]=7", a);
    a.set(3, 255);
    probe_all("set[3]=255", a);
    a.set(1, 256); // forces 8 -> 16
    probe_all("set[1]=256 (widen 16)", a);
    a.set(2, 70000); // forces 16 -> 32
    probe_all("set[2]=70000 (widen 32)", a);
    a.set(4, 0x1'0000'0000ull); // forces 32 -> 64
    probe_all("set[4]=2^32 (widen 64)", a);
    a.set(5, 0xFFFFFFFFFFFFFFFFull); // max, stays 64
    probe_all("set[5]=UINT64_MAX", a);

    // insert() in the middle, at the front, and at the end, after widening.
    a.insert(0, 42);
    probe_all("insert[0]=42", a);
    a.insert(a.size() / 2, 43);
    probe_all("insert[mid]=43", a);
    a.insert(a.size(), 44);
    probe_all("insert[end]=44", a);

    // erase from the front, middle and end.
    a.erase(0);
    probe_all("erase[0]", a);
    a.erase(a.size() / 2);
    probe_all("erase[mid]", a);
    a.erase(a.size() - 1);
    probe_all("erase[last]", a);

    // truncate() -- UNTRACED. Partial first, then to zero, which is the branch that
    // resets the width to 8 in BOTH the accessor and the header.
    a.truncate(a.size() - 2);
    probe_all("truncate(size-2)", a);
    a.truncate(3);
    probe_all("truncate(3)", a);
    a.truncate(0);
    probe_all("truncate(0) resets width", a);

    // The array must still be usable at width 8 after the reset.
    a.add(9);
    a.add(200);
    probe_all("after truncate(0), re-add", a);

    a.destroy();
}

// A dedicated widening walk: start narrow, push one value over each boundary and dump
// after every step, so an off-by-one in bit_width shows up as a width change at the
// wrong element.
void exercise_width_ladder()
{
    std::printf("=== width ladder ===\n");
    Allocator& alloc = Allocator::get_default();
    ArrayUnsigned a(alloc);
    a.create(0, 0);
    static const uint64_t ladder[] = {0,      1,       254,     255,        256,        257,
                                      65534,  65535,   65536,   65537,      0xFFFFFFFEull, 0xFFFFFFFFull,
                                      0x100000000ull,  0x100000001ull,      0x7FFFFFFFFFFFFFFFull,
                                      0x8000000000000000ull,                0xFFFFFFFFFFFFFFFFull};
    for (uint64_t v : ladder) {
        a.add(v);
        char tag[64];
        std::snprintf(tag, sizeof(tag), "add %llu", static_cast<unsigned long long>(v));
        dump(tag, a);
    }
    dump_values("ladder final", a);
    dump_bounds("ladder final", a);
    a.destroy();
}

// Insert-driven widening: every insertion position combined with a value that forces
// a widen, which is the path where insert() has to re-encode the elements on BOTH
// sides of the insertion point.
void exercise_insert_widening()
{
    std::printf("=== insert widening at every position ===\n");
    for (size_t pos = 0; pos <= 6; ++pos) {
        Allocator& alloc = Allocator::get_default();
        ArrayUnsigned a(alloc);
        a.create(0, 255);
        for (uint64_t v = 1; v <= 6; ++v)
            a.add(v * 3);
        a.insert(pos, 100000); // forces 8 -> 32 with elements on both sides
        char tag[64];
        std::snprintf(tag, sizeof(tag), "insert widen at %zu", pos);
        dump(tag, a);
        dump_values(tag, a);
        a.destroy();
    }
}

} // namespace

int main()
{
    for (uint64_t ub : {uint64_t(0), uint64_t(255), uint64_t(65535), uint64_t(0xFFFFFFFF),
                        uint64_t(0xFFFFFFFFFFFFFFFF)})
        exercise(ub);
    exercise_width_ladder();
    exercise_insert_widening();
    std::printf("done\n");
    return 0;
}
