// Differential driver for upstream/src/realm/array_timestamp.cpp.
//
// This unit is byte-visible and UNTRACED: no trace stores a Timestamp, because the
// trace schema is int/string/double/bool. So `make verify` proves the link is intact
// and nothing else regressed, and nothing about whether this port is correct. This
// driver is the entire evidence.
//
// What it has to cover, and why each matters:
//
//   * the null encoding. m_seconds is an ArrayIntNull, which stores a sentinel in
//     element 0 and shifts every index by one. Writing a value equal to the current
//     sentinel forces avoid_null_collision to pick a new one and rewrite the array --
//     so the driver deliberately stores values that collide.
//   * the seconds/nanoseconds split. Every comparison narrows on seconds first and
//     breaks ties on nanoseconds, so the interesting cases are equal seconds with
//     differing nanoseconds, and negative values on either side.
//   * six find_first specialisations whose null handling differs: Greater/Less return
//     not_found for a null needle, GreaterEqual/LessEqual/Equal search for nulls, and
//     NotEqual searches for non-nulls.
//
// Build and run via migration/checks/run_array_timestamp_differential.sh.

#include <realm/array_timestamp.hpp>
#include <realm/alloc.hpp>

#include <cstdint>
#include <cstdio>
#include <vector>

using namespace realm;

namespace {

Timestamp ts(int64_t s, int32_t n)
{
    return Timestamp(s, n);
}

void dump(const char* tag, const ArrayTimestamp& a)
{
    std::printf("%-26s size=%zu", tag, a.size());
    // Array is a *private* base here, so get_mem()/destroy() are inaccessible; get_ref()
    // is re-exported with a using-declaration, so the header comes via the allocator.
    const char* header = Allocator::get_default().translate(a.get_ref());
    std::printf(" hdr=");
    for (int i = 0; i < 8; ++i)
        std::printf("%02x", (unsigned char)header[i]);
    std::printf("\n");
    for (size_t i = 0; i < a.size(); ++i) {
        Timestamp v = a.get(i);
        if (v.is_null())
            std::printf("%-26s   [%zu] null\n", tag, i);
        else
            std::printf("%-26s   [%zu] s=%lld n=%d\n", tag, i, (long long)v.get_seconds(), v.get_nanoseconds());
    }
}

// All six comparators over one needle, printed together so a single wrong branch shows
// up as one changed column rather than a whole block.
void probe(const char* tag, const ArrayTimestamp& a, Timestamp needle)
{
    size_t n = a.size();
    std::printf("%-26s needle=%s%lld/%d  eq=%zu ne=%zu lt=%zu le=%zu gt=%zu ge=%zu\n", tag,
                needle.is_null() ? "null " : "", needle.is_null() ? 0LL : (long long)needle.get_seconds(),
                needle.is_null() ? 0 : needle.get_nanoseconds(), a.find_first<Equal>(needle, 0, n),
                a.find_first<NotEqual>(needle, 0, n), a.find_first<Less>(needle, 0, n),
                a.find_first<LessEqual>(needle, 0, n), a.find_first<Greater>(needle, 0, n),
                a.find_first<GreaterEqual>(needle, 0, n));
}

} // namespace

int main()
{
    std::setvbuf(stdout, nullptr, _IONBF, 0);
    Allocator& alloc = Allocator::get_default();

    // ---------------------------------------------------------------------
    // 1. create + insert, covering nulls, zero, negatives, and the nanosecond
    //    tie-break (equal seconds, different nanoseconds).
    // ---------------------------------------------------------------------
    ArrayTimestamp a(alloc);
    a.create();
    dump("empty", a);

    const Timestamp vals[] = {
        ts(0, 0),          ts(0, 1),         ts(0, -1),        ts(1, 0),
        ts(-1, 0),         ts(100, 500),     ts(100, 499),     ts(100, 501),
        ts(-100, -500),    ts(1'000'000, 0), Timestamp{},      ts(0, 999'999'999),
        ts(-1, 999'999'999),
    };
    for (size_t i = 0; i < sizeof(vals) / sizeof(vals[0]); ++i) {
        a.insert(a.size(), vals[i]);
    }
    dump("inserted", a);

    // ---------------------------------------------------------------------
    // 2. Insert at the front and middle, which shifts the underlying arrays.
    // ---------------------------------------------------------------------
    a.insert(0, ts(-5, 5));
    dump("insert_front", a);
    a.insert(4, Timestamp{});
    dump("insert_mid_null", a);
    a.insert(a.size(), ts(7, 7));
    dump("insert_end", a);

    // ---------------------------------------------------------------------
    // 3. set(), including to and from null, and the sentinel-collision case.
    //
    //    ArrayIntNull picks a sentinel that is not present in the data. Storing a
    //    value equal to it must trigger avoid_null_collision, which re-picks and
    //    rewrites. Sweeping a wide range of seconds values is the cheap way to hit
    //    whatever sentinel it happens to have chosen.
    // ---------------------------------------------------------------------
    a.set(0, ts(42, 42));
    dump("set_front", a);
    a.set(2, Timestamp{});
    dump("set_to_null", a);
    a.set(2, ts(3, 3));
    dump("set_from_null", a);
    for (int64_t s = -20; s <= 20; ++s) {
        a.set(1, ts(s, int32_t(s)));
    }
    dump("set_sweep", a);
    // Large magnitudes, where a naive sentinel choice is most likely to collide.
    for (int64_t s : {int64_t(0), int64_t(1) << 20, -(int64_t(1) << 20), int64_t(1) << 40, -(int64_t(1) << 40)}) {
        a.set(3, ts(s, 0));
    }
    dump("set_bigmag", a);

    // ---------------------------------------------------------------------
    // 4. All six comparators over a spread of needles, including nulls and
    //    values that tie on seconds.
    // ---------------------------------------------------------------------
    const Timestamp needles[] = {
        ts(0, 0),      ts(0, 1),        ts(0, -1),   ts(100, 500), ts(100, 499),
        ts(100, 501),  ts(-100, -500),  ts(7, 7),    ts(42, 42),   ts(1'000'000, 0),
        ts(999, 999),  Timestamp{},     ts(-1, 0),   ts(1, 0),
    };
    for (auto n : needles)
        probe("probe", a, n);

    // Ranged searches, including empty and single-element ranges.
    {
        size_t n = a.size();
        std::printf("ranged eq(0,0)=%zu eq(1,1)=%zu eq(0,1)=%zu eq(n-1,n)=%zu\n",
                    a.find_first<Equal>(ts(42, 42), 0, 0), a.find_first<Equal>(ts(42, 42), 1, 1),
                    a.find_first<Equal>(ts(42, 42), 0, 1), a.find_first<Equal>(ts(42, 42), n - 1, n));
    }

    // ---------------------------------------------------------------------
    // 5. find_first_in_range across boundaries: fully inside, touching each end,
    //    inverted, and empty.
    // ---------------------------------------------------------------------
    {
        size_t n = a.size();
        const struct {
            Timestamp from, to;
        } ranges[] = {
            {ts(0, 0), ts(0, 0)},     {ts(0, 0), ts(1, 0)},       {ts(-1, 0), ts(1, 0)},
            {ts(100, 499), ts(100, 501)}, {ts(100, 500), ts(100, 500)}, {ts(-1000, 0), ts(1000, 0)},
            {ts(1, 0), ts(-1, 0)},    {ts(1'000'000, 0), ts(1'000'000, 0)},
        };
        for (auto& r : ranges)
            std::printf("in_range [%lld/%d .. %lld/%d] = %zu\n", (long long)r.from.get_seconds(),
                        r.from.get_nanoseconds(), (long long)r.to.get_seconds(), r.to.get_nanoseconds(),
                        a.find_first_in_range(r.from, r.to, 0, n));
    }

    // ---------------------------------------------------------------------
    // 6. Round-trip through init_from_mem, which re-attaches both sub-arrays
    //    from the parent refs rather than constructing them.
    // ---------------------------------------------------------------------
    {
        ref_type r = a.get_ref();
        ArrayTimestamp b(alloc);
        b.init_from_mem(MemRef(alloc.translate(r), r, alloc));
        dump("reattached", b);
        probe("reattached.probe", b, ts(42, 42));
        probe("reattached.probe", b, Timestamp{});
    }

    // Array::destroy() is inaccessible through the private base; the process exits.
    std::printf("done\n");
    return 0;
}
