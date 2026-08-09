// Differential driver for upstream/src/realm/array_blob.cpp.
//
// `make diff-test` already judges this unit well: an element-width bug injected into
// the Rust fails all five traces. But breakpoint counts show the traces reach exactly
// one of the four exported functions:
//
//     ArrayBlob::replace     3 / 46 / 13 / 29 / 19 hits
//     Array::blob_replace    0
//     ArrayBlob::get_at      0
//     ArrayBlob::verify      0   (body is entirely #ifdef REALM_DEBUG -- empty here)
//
// The two unreached ones that matter both need a *split* blob: one larger than
// ArrayBlob::max_binary_size (16,777,200 bytes), where the root stops holding bytes and
// starts holding refs to child blobs. No trace stores 16 MB of anything.
//
// Driving the split by hand through ArrayBlob was a dead end, and it is worth recording
// why: once the root's context flag is set, m_size counts refs rather than bytes, so
// ArrayBlob::add and destroy_deep become the wrong receivers. The pure-C++ driver
// segfaulted, then aborted inside realm's own asserts. Those are states realm never
// creates. The split is driven here through ArrayBigBlobs -- realm's actual caller --
// which produces it correctly.
//
// Deliberately not compared: ref values. They are allocator addresses, not structure.
// Everything printed is size, flag, content or header.
//
// Build and run via migration/checks/run_array_blob_differential.sh.

#include <realm/array_blob.hpp>
#include <realm/array_blobs_big.hpp>
#include <realm/array.hpp>
#include <realm/alloc.hpp>

#include <cstdint>
#include <cstdio>
#include <cstring>
#include <vector>

using namespace realm;

namespace {

// Deterministic byte source, so both stacks see identical multi-megabyte input without
// a literal.
uint8_t byte_at(size_t i)
{
    uint64_t x = i * 0x9E3779B97F4A7C15ull + 0x123456789ABCDEFull;
    x ^= x >> 29;
    x *= 0xBF58476D1CE4E5B9ull;
    x ^= x >> 32;
    return uint8_t(x);
}

std::vector<char> make_chunk(size_t start, size_t n)
{
    std::vector<char> v(n);
    for (size_t i = 0; i < n; ++i)
        v[i] = char(byte_at(start + i));
    return v;
}

uint32_t checksum(const char* p, size_t n)
{
    uint32_t h = 2166136261u;
    for (size_t i = 0; i < n; ++i) {
        h ^= uint8_t(p[i]);
        h *= 16777619u;
    }
    return h;
}

void dump_node(const char* tag, const ArrayBlob& b)
{
    MemRef mem = b.get_mem();
    const char* header = mem.get_addr();
    std::printf("%-28s size=%zu blob_size=%zu ctx=%d hdr=", tag, b.size(), b.blob_size(),
                int(b.get_context_flag()));
    for (int i = 0; i < 8; ++i)
        std::printf("%02x", (unsigned char)header[i]);
    std::printf("\n");
}

// Walk one blob of an ArrayBigBlobs through get_at. For a split blob this is the only
// way to read it, and the chunk sequence is the observable that changes if the
// child-walking arithmetic is wrong.
void walk_big(const char* tag, ArrayBigBlobs& bb, size_t ndx, size_t expected)
{
    size_t pos = 0;
    int step = 0;
    size_t seen = 0;
    uint32_t running = 2166136261u;
    while (true) {
        size_t before = pos;
        BinaryData c = bb.get_at(ndx, pos);
        if (c.size() == 0)
            break;
        for (size_t i = 0; i < c.size(); ++i) {
            running ^= uint8_t(c.data()[i]);
            running *= 16777619u;
        }
        seen += c.size();
        if (step < 6)
            std::printf("%-28s step=%d pos_in=%zu size=%zu csum=%u pos_out=%zu\n", tag, step, before, c.size(),
                        checksum(c.data(), c.size()), pos);
        ++step;
        if (pos == 0)
            break;
        if (step > 100000)
            break; // a port that never terminates must not hang the check
    }
    std::printf("%-28s steps=%d seen=%zu csum=%u expected=%zu match=%d\n", tag, step, seen, running, expected,
                int(seen == expected));
}


// get_at from offsets that land INSIDE a child, not just at child boundaries.
//
// walk_big always enters with offset 0 relative to the child it lands in, so
// `sz = current_size - offset` and `sz = current_size` are indistinguishable there.
// A negative control proved that blind spot: corrupting that subtraction left the
// walk output byte-identical. These probes are what make the subtraction observable.
void probe_inside(const char* tag, ArrayBigBlobs& bb, size_t ndx, const std::vector<size_t>& positions)
{
    for (size_t p : positions) {
        size_t pos = p;
        BinaryData c = bb.get_at(ndx, pos);
        std::printf("%-28s at=%-12zu size=%-12zu csum=%-12u next=%zu\n", tag, p, c.size(),
                    c.size() ? checksum(c.data(), c.size()) : 0u, pos);
    }
}

} // namespace

int main()
{
    // Unbuffered: on a crash the last line printed is the last line reached.
    std::setvbuf(stdout, nullptr, _IONBF, 0);

    Allocator& alloc = Allocator::get_default();
    const size_t MAX = ArrayBlob::max_binary_size;
    std::printf("max_binary_size=%zu\n", MAX);

    // ---------------------------------------------------------------------
    // 1. Small blob driven directly: ArrayBlob::replace in all four shapes, and
    //    get_at's non-context-flag branch, which no trace reaches either.
    // ---------------------------------------------------------------------
    {
        ArrayBlob b(alloc);
        b.create();
        auto c = make_chunk(0, 1000);
        b.add(c.data(), c.size());
        dump_node("small", b);

        for (size_t p : {size_t(0), size_t(1), size_t(499), size_t(999), size_t(1000), size_t(1001), size_t(5000)}) {
            size_t pos = p;
            BinaryData d = b.get_at(pos);
            std::printf("small.get_at at=%-6zu size=%-6zu csum=%-12u next=%zu\n", p, d.size(),
                        d.size() ? checksum(d.data(), d.size()) : 0u, pos);
        }

        auto r = make_chunk(9000, 10);
        b.replace(10, 20, r.data(), r.size()); // same size: no gap move
        dump_node("small.replace_same", b);
        b.replace(0, 5, r.data(), r.size()); // grow: copy_backward expands the gap
        dump_node("small.replace_grow", b);
        b.replace(100, 130, r.data(), r.size()); // shrink: safe_copy_n closes the gap
        dump_node("small.replace_shrink", b);
        b.replace(b.blob_size(), b.blob_size(), r.data(), r.size(), true); // append + zero term
        dump_node("small.append_zeroterm", b);

        size_t pos = 0;
        BinaryData d = b.get_at(pos);
        std::printf("small.after size=%zu csum=%u\n", d.size(), checksum(d.data(), d.size()));
        b.destroy_deep();
    }

    // ---------------------------------------------------------------------
    // 2. Sizes across the split boundary, through ArrayBigBlobs.
    //    MAX must not split; MAX+1 must.
    // ---------------------------------------------------------------------
    for (size_t n : {size_t(1), MAX - 1, MAX, MAX + 1, MAX + 1000, 2 * MAX + 12345}) {
        ArrayBigBlobs bb(alloc, false);
        bb.create();
        auto data = make_chunk(n, n);
        bb.add(BinaryData(data.data(), n), false);

        char tag[80];
        std::snprintf(tag, sizeof(tag), "big.%zu", n);
        BinaryData whole = bb.get(0);
        // For a split blob get() returns empty and the caller must use get_at. Captured
        // explicitly, because it is exactly the difference between the two branches.
        std::printf("%-28s count=%zu get0_size=%zu split=%d\n", tag, bb.size(), whole.size(),
                    int(whole.size() == 0 && n > 0));
        walk_big(tag, bb, 0, n);
        probe_inside(tag, bb, 0,
                     {0, 1, 2, 7, n / 3, n / 2, MAX / 2, MAX - 2, MAX - 1, MAX, MAX + 1, MAX + 2,
                      n > 3 ? n - 3 : 0, n > 1 ? n - 1 : 0, n, n + 1});
        bb.destroy_deep();
    }

    // ---------------------------------------------------------------------
    // 3. Several blobs in one node, mixing split and unsplit, then walk each.
    // ---------------------------------------------------------------------
    {
        ArrayBigBlobs bb(alloc, false);
        bb.create();
        const size_t sizes[] = {10, MAX + 1, 4096, MAX - 1, 1};
        for (size_t i = 0; i < 5; ++i) {
            auto d = make_chunk(i * 7919, sizes[i]);
            bb.add(BinaryData(d.data(), sizes[i]), false);
        }
        std::printf("mixed count=%zu\n", bb.size());
        for (size_t i = 0; i < 5; ++i) {
            char tag[80];
            std::snprintf(tag, sizeof(tag), "mixed[%zu].%zu", i, sizes[i]);
            std::printf("%-28s get_size=%zu\n", tag, bb.get(i).size());
            walk_big(tag, bb, i, sizes[i]);
            probe_inside(tag, bb, i, {0, 1, sizes[i] / 2, MAX / 2, MAX - 1, MAX, MAX + 1,
                                      sizes[i] > 1 ? sizes[i] - 1 : 0, sizes[i]});
        }
        bb.destroy_deep();
    }

    std::printf("done\n");
    return 0;
}
