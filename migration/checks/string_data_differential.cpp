// Differential driver for string_data.cpp.
//
// Compiled twice by run_string_data_differential.sh — once against the oracle's
// string_data.cpp.o, once against the Rust staticlib — and the two dumps must be
// byte-identical.
//
// Why this exists: murmur2_or_cityhash feeds string-index keys, so a divergent hash
// puts different bytes in a .realm. But none of the traces in harness/traces/ builds a
// string index, so `make diff-test` never evaluates these functions. Without this
// driver the unit would be "green" on evidence that never touched it.
//
// The corpus deliberately straddles every length branch of cityhash_64
// (0, 1..3, 4..8, 9..16, 17..32, 33..64, >64 and its 64-byte loop), because those
// boundaries are where a transcription error hides.

#include <cstdio>
#include <cstdint>
#include <cstddef>
#include <string>
#include <vector>

#include <realm/string_data.hpp>

using realm::StringData;

// matchlike/matchlike_ins are private statics on StringData, reachable in-tree only
// via StringData::like() and unicode.cpp's string_like_ins(). Binding the mangled
// symbols directly with asm labels lets the driver call exactly the definitions under
// test, without dragging unicode.cpp into the link and without depending on an inline
// wrapper that might be optimised into something else.
//
// The label is verbatim on Darwin, so the leading underscore is written explicitly.
bool probe_matchlike(const StringData&, const StringData&) noexcept
    asm("__ZN5realm10StringData9matchlikeERKS0_S2_");
bool probe_matchlike_ins(const StringData&, const StringData&, const StringData&) noexcept
    asm("__ZN5realm10StringData13matchlike_insERKS0_S2_S2_");

namespace {

// Deterministic pseudo-random bytes. Not std::rand: this must produce the same corpus
// in both builds and on any host, or the comparison is meaningless.
std::string make_bytes(size_t len, uint32_t seed)
{
    std::string s;
    s.reserve(len);
    uint32_t x = seed * 2654435761u + 1u;
    for (size_t i = 0; i < len; ++i) {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        s.push_back(static_cast<char>(x & 0xff));
    }
    return s;
}

void dump_hashes(const std::string& label, const std::string& s)
{
    const auto* p = reinterpret_cast<const unsigned char*>(s.data());
    const size_t n = s.size();
    std::printf("%-28s len=%-5zu murmur2_32=%08x cityhash_64=%016llx combined=%016llx\n", label.c_str(), n,
                static_cast<unsigned>(realm::murmur2_32(p, n)),
                static_cast<unsigned long long>(realm::cityhash_64(p, n)),
                static_cast<unsigned long long>(realm::murmur2_or_cityhash(p, n)));
}

void dump_like(const char* text, const char* pattern)
{
    StringData t{text};
    StringData p{pattern};
    std::printf("like  text=%-18s pattern=%-18s -> %d\n", text, pattern, int(probe_matchlike(t, p)));
}

void dump_like_ins(const char* text, const char* upper, const char* lower)
{
    StringData t{text};
    StringData u{upper};
    StringData l{lower};
    std::printf("likei text=%-18s upper=%-18s lower=%-18s -> %d\n", text, upper, lower,
                int(probe_matchlike_ins(t, u, l)));
}

} // namespace

int main()
{
    std::printf("=== hashes: every length from 0 to 130 ===\n");
    // Exhaustive over the small lengths, which covers all of cityhash's branch
    // boundaries and murmur2's 3/2/1-byte tail fallthrough.
    for (size_t len = 0; len <= 130; ++len) {
        dump_hashes("seq/" + std::to_string(len), make_bytes(len, 1));
    }

    std::printf("\n=== hashes: longer inputs, exercising the 64-byte loop ===\n");
    for (size_t len : {191u, 192u, 193u, 255u, 256u, 257u, 1024u, 4096u}) {
        dump_hashes("long/" + std::to_string(len), make_bytes(len, 7));
    }

    std::printf("\n=== hashes: high-bit and zero bytes ===\n");
    // A hash that widened a signed char somewhere diverges here and nowhere else.
    for (size_t len : {1u, 3u, 4u, 7u, 8u, 15u, 16u, 33u, 65u}) {
        std::string all_ff(len, char(0xff));
        std::string all_00(len, char(0x00));
        std::string all_80(len, char(0x80));
        dump_hashes("0xff/" + std::to_string(len), all_ff);
        dump_hashes("0x00/" + std::to_string(len), all_00);
        dump_hashes("0x80/" + std::to_string(len), all_80);
    }

    std::printf("\n=== matchlike ===\n");
    const char* texts[] = {"", "a", "abc", "aaa", "abcabc", "xayaz", "hello world", "\xc3\xa9", "\xf0\x9f\x98\x80"};
    const char* patterns[] = {"",   "a",   "*",     "?",    "a*",   "*c",   "a*c",  "*a",
                              "*a*a*", "*a*a*a*", "?bc", "a?c", "??", "*?*", "abc", "*world", "hello*"};
    for (const char* t : texts) {
        for (const char* p : patterns) {
            dump_like(t, p);
        }
    }

    std::printf("\n=== matchlike_ins ===\n");
    dump_like_ins("Hello", "HELLO", "hello");
    dump_like_ins("HeLLo", "HELLO", "hello");
    dump_like_ins("hello", "HELLO", "hello");
    dump_like_ins("Hellx", "HELLO", "hello");
    dump_like_ins("HeLLo World", "*WORLD", "*world");
    dump_like_ins("", "", "");
    dump_like_ins("abc", "?B?", "?b?");

    return 0;
}
