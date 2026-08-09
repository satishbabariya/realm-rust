// Differential driver for upstream/src/realm/array_blobs_small.cpp.
//
// `make diff-test` reaches this unit -- ArraySmallBlobs::insert is hit 46 times and set
// 4 times across the five traces -- but it reaches it *shallowly*. Two bugs injected
// into the landed Rust:
//
//   zero terminator not counted in the stored size   -> diff-test FAILS (1 trace)
//   later offsets not shifted on a mid-list insert   -> diff-test PASSES
//
// The second is the coordination bug this unit exists to get right, and the traces miss
// it because they only ever *append*: with ndx == size(), the adjust range
// [ndx+1, size()) is empty, so skipping it changes nothing. Nothing in the trace schema
// inserts into the middle of a string column, erases from it, or reads a legacy node.
//
// This driver covers what the traces do not: insertion and erasure at the front and
// middle, set() growing and shrinking an element, nulls, find_first, the static
// header-only get(), and get_string_legacy.
//
// Build and run via migration/checks/run_array_blobs_small_differential.sh.

#include <realm/array_blobs_small.hpp>
#include <realm/alloc.hpp>

#include <cstdint>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>

using namespace realm;

namespace {

uint32_t checksum(const char* p, size_t n)
{
    uint32_t h = 2166136261u;
    for (size_t i = 0; i < n; ++i) {
        h ^= uint8_t(p[i]);
        h *= 16777619u;
    }
    return h;
}

// Print everything that distinguishes one small-blob node from another: the element
// count, each element's bytes and null flag, and the raw header of the top node. The
// offsets array is not printed directly -- it has no public accessor -- but every
// element's size and content depends on it, so a wrong offset shows up as wrong bytes.
void dump(const char* tag, const ArraySmallBlobs& a)
{
    std::printf("%-30s size=%zu", tag, a.size());
    MemRef mem = a.get_mem();
    const char* header = mem.get_addr();
    std::printf(" hdr=");
    for (int i = 0; i < 8; ++i)
        std::printf("%02x", (unsigned char)header[i]);
    std::printf("\n");
    for (size_t i = 0; i < a.size(); ++i) {
        BinaryData b = a.get(i);
        std::printf("%-30s   [%zu] null=%d size=%zu csum=%u\n", tag, i, int(b.is_null()), b.size(),
                    b.is_null() ? 0u : checksum(b.data(), b.size()));
    }
}

BinaryData bd(const std::string& s)
{
    return BinaryData(s.data(), s.size());
}

} // namespace

int main()
{
    std::setvbuf(stdout, nullptr, _IONBF, 0);
    Allocator& alloc = Allocator::get_default();

    // ---------------------------------------------------------------------
    // 1. create_array at several seed sizes, with a null and a non-null seed.
    //    The seed decides what m_nulls is filled with, which is a byte-level
    //    difference invisible to size() alone.
    // ---------------------------------------------------------------------
    for (size_t n : {size_t(0), size_t(1), size_t(5)}) {
        for (int null_seed = 0; null_seed < 2; ++null_seed) {
            BinaryData seed = null_seed ? BinaryData() : BinaryData("", 0);
            MemRef mem = ArraySmallBlobs::create_array(n, alloc, seed);
            ArraySmallBlobs a(alloc);
            a.init_from_mem(mem);
            char tag[64];
            std::snprintf(tag, sizeof(tag), "create.%zu.null%d", n, null_seed);
            dump(tag, a);
            a.destroy_deep();
        }
    }

    // ---------------------------------------------------------------------
    // 2. Append-only, which is what the traces already cover. Included so a
    //    regression there shows up here too rather than only in the gate.
    // ---------------------------------------------------------------------
    {
        MemRef mem = ArraySmallBlobs::create_array(0, alloc, BinaryData("", 0));
        ArraySmallBlobs a(alloc);
        a.init_from_mem(mem);
        const char* vals[] = {"", "a", "bb", "ccc", "dddd"};
        for (const char* v : vals)
            a.add(BinaryData(v, std::strlen(v)), true); // zero-terminated, as strings are
        dump("append.zeroterm", a);
        a.add(BinaryData(), false); // a null
        dump("append.null", a);
        a.add(BinaryData("xy", 2), false); // binary, no terminator
        dump("append.binary", a);
        a.destroy_deep();
    }

    // ---------------------------------------------------------------------
    // 3. THE PATH THE TRACES MISS: insert at front and middle, so the adjust
    //    range [ndx+1, size()) is non-empty and every later offset must shift.
    // ---------------------------------------------------------------------
    {
        MemRef mem = ArraySmallBlobs::create_array(0, alloc, BinaryData("", 0));
        ArraySmallBlobs a(alloc);
        a.init_from_mem(mem);
        for (const char* v : {"one", "two", "three", "four"})
            a.add(BinaryData(v, std::strlen(v)), true);
        dump("mid.base", a);

        a.insert(0, bd("FRONT"), true); // front: every offset shifts
        dump("mid.insert_front", a);
        a.insert(3, bd("MIDDLE"), true); // middle
        dump("mid.insert_middle", a);
        a.insert(a.size(), bd("END"), true); // end: the append case, for contrast
        dump("mid.insert_end", a);
        a.insert(2, BinaryData(), false); // a null in the middle
        dump("mid.insert_null", a);
        a.insert(1, BinaryData("", 0), true); // empty, still zero-terminated
        dump("mid.insert_empty", a);
        a.destroy_deep();
    }

    // ---------------------------------------------------------------------
    // 4. set(): shrinking, growing and same-size, at front, middle and end.
    //    Each one drives a different branch of ArrayBlob::replace and a
    //    different sign of the offset adjustment.
    // ---------------------------------------------------------------------
    {
        MemRef mem = ArraySmallBlobs::create_array(0, alloc, BinaryData("", 0));
        ArraySmallBlobs a(alloc);
        a.init_from_mem(mem);
        for (const char* v : {"aaaa", "bbbb", "cccc", "dddd", "eeee"})
            a.add(BinaryData(v, std::strlen(v)), true);
        dump("set.base", a);

        a.set(0, bd("SHORT"), true);
        dump("set.front_grow", a);
        a.set(2, bd("x"), true); // shrink
        dump("set.mid_shrink", a);
        a.set(3, bd("dddd"), true); // same size
        dump("set.mid_same", a);
        a.set(a.size() - 1, bd("a much longer value than before"), true);
        dump("set.end_grow", a);
        a.set(1, BinaryData(), false); // set to null
        dump("set.to_null", a);
        a.set(1, bd("back"), true); // and back again
        dump("set.from_null", a);
        a.destroy_deep();
    }

    // ---------------------------------------------------------------------
    // 5. erase() at front, middle and end.
    // ---------------------------------------------------------------------
    {
        MemRef mem = ArraySmallBlobs::create_array(0, alloc, BinaryData("", 0));
        ArraySmallBlobs a(alloc);
        a.init_from_mem(mem);
        for (const char* v : {"q", "ww", "eee", "rrrr", "ttttt", "yyyyyy"})
            a.add(BinaryData(v, std::strlen(v)), true);
        a.add(BinaryData(), false);
        dump("erase.base", a);
        a.erase(0);
        dump("erase.front", a);
        a.erase(2);
        dump("erase.middle", a);
        a.erase(a.size() - 1);
        dump("erase.last", a);
        while (a.size())
            a.erase(0);
        dump("erase.empty", a);
        a.destroy_deep();
    }

    // ---------------------------------------------------------------------
    // 6. find_first, string and binary, hit and miss, plus the null search.
    //    is_string adds one to the length being matched, so a port that drops
    //    it misses every string.
    // ---------------------------------------------------------------------
    {
        MemRef mem = ArraySmallBlobs::create_array(0, alloc, BinaryData("", 0));
        ArraySmallBlobs a(alloc);
        a.init_from_mem(mem);
        for (const char* v : {"alpha", "beta", "gamma", "beta"})
            a.add(BinaryData(v, std::strlen(v)), true);
        a.add(BinaryData(), false);
        a.add(BinaryData("bin", 3), false);

        static const char* needles[] = {"alpha", "beta", "gamma", "delta", "", "bin", "b"};
        for (const char* nd : needles) {
            size_t s = std::strlen(nd);
            std::printf("find str(%-6s)=%zu  bin(%-6s)=%zu\n", nd,
                        a.find_first(BinaryData(nd, s), true, 0, npos), nd,
                        a.find_first(BinaryData(nd, s), false, 0, npos));
        }
        std::printf("find null=%zu\n", a.find_first(BinaryData(), true, 0, npos));
        // ranged searches, including empty and reversed-at-the-edge ranges
        std::printf("find ranged=%zu,%zu,%zu\n", a.find_first(bd("beta"), true, 2, npos),
                    a.find_first(bd("beta"), true, 0, 1), a.find_first(bd("beta"), true, 3, 3));

        // 7. The static header-only get(), which constructs no accessor.
        MemRef m2 = a.get_mem();
        for (size_t i = 0; i < a.size(); ++i) {
            BinaryData b = ArraySmallBlobs::get(m2.get_addr(), i, alloc);
            std::printf("static_get[%zu] null=%d size=%zu csum=%u\n", i, int(b.is_null()), b.size(),
                        b.is_null() ? 0u : checksum(b.data(), b.size()));
        }

        // get_string_legacy is PRIVATE and has no public caller reachable from here, so
        // it cannot be driven through this API. It is exported and linked, so something
        // in a wider build calls it; within this harness it stays unverified, and saying
        // so is better than a probe that silently tests nothing.
        a.destroy_deep();
    }

    std::printf("done\n");
    return 0;
}
