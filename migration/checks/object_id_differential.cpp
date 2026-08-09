// Differential driver for upstream/src/realm/object_id.cpp.
//
// ObjectId is a 12-byte on-disk value and its internal byte order is a file-format
// decision -- seconds and sequence big-endian, machine/process id little-endian. But no
// trace stores one, so `make verify` proves only that the link is intact. This is the
// evidence.
//
// Two things here are deliberately NOT compared as absolute values, because they are
// random in both stacks: the machine/process id drawn by the generator's static state,
// and the starting point of the sequence counter. What is compared is everything
// derived from them that is *supposed* to be deterministic -- the counter's delta, the
// field positions, and the fact that identical inputs give identical bytes.

#include <realm/object_id.hpp>
#include <realm/string_data.hpp>

#include <cstdio>
#include <cstring>
#include <string>
#include <vector>

using namespace realm;

namespace {

void dump(const char* tag, const ObjectId& id)
{
    auto bytes = id.to_bytes();
    std::printf("%-30s bytes=", tag);
    for (size_t i = 0; i < bytes.size(); ++i)
        std::printf("%02x", bytes[i]);
    std::string s = id.to_string();
    std::printf(" to_string=[%zu,cap=%zu]%s ts=%lld hash_nonzero=%d\n", s.size(), s.capacity(), s.c_str(),
                (long long)id.get_timestamp().get_seconds(), int(id.hash() != 0));
}

} // namespace

int main()
{
    // 1. is_valid_str over lengths and alphabets. The C++ requires exactly 24 and uses
    //    isxdigit, so case-mixing is accepted and one bad byte is not.
    static const char* strs[] = {"",
                                 "0",
                                 "000102030405060708090a0",     // 23
                                 "000102030405060708090a0b",    // 24 ok
                                 "000102030405060708090a0bc",   // 25
                                 "ABCDEFabcdef0123456789AB",    // 24 mixed case
                                 "000102030405060708090a0g",    // non-hex at end
                                 "g00102030405060708090a0b",    // non-hex at start
                                 "0001020304050607 8090a0b",    // space
                                 "0001020304050607\t8090a0b"};
    for (const char* s : strs)
        std::printf("valid(%-26s)=%d\n", s, int(ObjectId::is_valid_str(StringData(s, std::strlen(s)))));

    // 2. Round-trip every valid string through the parsing constructor, to_bytes and
    //    to_string. to_string always makes exactly 24 chars, which is past libc++'s
    //    short-string limit, so its capacity is part of what is being compared.
    static const char* ids[] = {"000000000000000000000000", "ffffffffffffffffffffffff",
                                "0123456789abcdef01234567", "ABCDEFABCDEFABCDEFABCDEF",
                                "00000000000000000000ffff", "ffff00000000000000000000",
                                "7fffffff0000000000000000", "80000000ffffffffffffffff"};
    for (const char* s : ids) {
        ObjectId id{StringData(s, 24)};
        dump(s, id);
        // to_string must be the lowercase normalisation of the input.
        std::string round = id.to_string();
        ObjectId again{StringData(round.data(), round.size())};
        std::printf("  reparse_equal=%d\n", int(std::memcmp(id.to_bytes().data(), again.to_bytes().data(), 12) == 0));
    }

    // 3. The Timestamp constructor with fixed machine/process ids. This is the function
    //    that decides the on-disk byte order, so the raw bytes are the comparison.
    //    Only bytes 9..11 (the sequence) vary between runs; they are printed separately.
    static const long secs[] = {0, 1, 255, 256, 65535, 65536, 0x7fffffff, 0x80000000L, 0xffffffffL};
    static const int mids[] = {0, 1, 0x112233, 0x7fffff, -1};
    static const int pids[] = {0, 1, 0x4455, 0x7fff, -1};
    for (long s : secs)
        for (int m : mids)
            for (int p : pids) {
                ObjectId id{Timestamp(int64_t(s), 0), m, p};
                auto b = id.to_bytes();
                std::printf("ts sec=%-11ld mid=%-9d pid=%-7d head=", s, m, p);
                for (int i = 0; i < 9; ++i) // bytes 0..8 are fully determined
                    std::printf("%02x", b[i]);
                std::printf(" ts_back=%lld\n", (long long)id.get_timestamp().get_seconds());
            }

    // 4. The sequence counter. Absolute value is random per process; the delta is not.
    {
        auto seq_of = [](const ObjectId& id) {
            auto b = id.to_bytes();
            return (uint32_t(b[9]) << 16) | (uint32_t(b[10]) << 8) | uint32_t(b[11]);
        };
        ObjectId a{Timestamp(0, 0), 0, 0};
        std::vector<uint32_t> deltas;
        uint32_t prev = seq_of(a);
        for (int i = 0; i < 16; ++i) {
            ObjectId n{Timestamp(0, 0), 0, 0};
            uint32_t cur = seq_of(n);
            deltas.push_back((cur - prev) & 0xffffff);
            prev = cur;
        }
        std::printf("seq_deltas=");
        for (uint32_t d : deltas)
            std::printf("%u,", d);
        std::printf("\n");
    }

    // 5. The bytes constructor, and to_bytes as its inverse.
    {
        ObjectId::ObjectIdBytes raw{};
        for (size_t i = 0; i < raw.size(); ++i)
            raw[i] = uint8_t(i * 17 + 3);
        ObjectId id{raw};
        dump("from_bytes", id);
        std::printf("  bytes_roundtrip=%d\n", int(std::memcmp(id.to_bytes().data(), raw.data(), 12) == 0));
    }

    // 6. hash() over a spread of values. It routes into murmur2_or_cityhash, which is
    //    itself Rust now, so a regression there shows up here too.
    for (const char* s : ids) {
        ObjectId id{StringData(s, 24)};
        std::printf("hash(%s)=%zu\n", s, id.hash());
    }

    // 7. gen() cannot be compared by value -- its machine/process id and sequence are
    //    random in both stacks. What must hold is the structure: the timestamp field
    //    reads back as a plausible current time, and two ids made in the same second
    //    differ only in the sequence bytes.
    {
        ObjectId g1 = ObjectId::gen();
        ObjectId g2 = ObjectId::gen();
        auto b1 = g1.to_bytes(), b2 = g2.to_bytes();
        bool same_prefix = std::memcmp(b1.data(), b2.data(), 9) == 0;
        long long t1 = g1.get_timestamp().get_seconds();
        std::printf("gen structure: same_first9=%d ts_plausible=%d strlen=%zu\n", int(same_prefix),
                    int(t1 > 1600000000LL && t1 < 4000000000LL), g1.to_string().size());
    }

    std::printf("done\n");
    return 0;
}
