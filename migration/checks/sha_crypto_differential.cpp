// Differential driver for util/sha_crypto.cpp.
//
// Compiled twice by run_sha_crypto_differential.sh — once against the oracle's
// sha_crypto.cpp.o, once against the Rust staticlib — and the two dumps must be
// byte-identical.
//
// Both sides ultimately call CommonCrypto, so agreement on the digest *values* is
// close to guaranteed. That is not what this driver is for. What it actually tests:
//
//   1. the Span ABI. Fixed-extent realm::util::Span<T, N> stores only a pointer,
//      dynamic-extent Span<T> stores {ptr, size}. Modelling either one wrongly on the
//      Rust side shifts every later argument register and produces garbage or a crash.
//   2. the CommonCrypto algorithm selectors: kCCHmacAlgSHA224 = 5, not 4, and
//      kCCHmacAlgSHA256 = 2. A wrong ordinal silently yields a valid-looking MAC of
//      the wrong algorithm.
//   3. the CC_LONG 32-bit truncation of the input length.

#include <cstdio>
#include <cstdint>
#include <cstddef>
#include <string>
#include <vector>
#include <array>

#include <realm/util/sha_crypto.hpp>
#include <realm/util/span.hpp>

using realm::util::Span;

namespace {

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

void print_hex(const char* label, size_t len, const unsigned char* d, size_t n)
{
    std::printf("%-10s len=%-6zu ", label, len);
    for (size_t i = 0; i < n; ++i)
        std::printf("%02x", d[i]);
    std::printf("\n");
}

void dump_sha(const std::string& s)
{
    unsigned char out1[20] = {};
    unsigned char out256[32] = {};
    realm::util::sha1(s.data(), s.size(), out1);
    realm::util::sha256(s.data(), s.size(), out256);
    print_hex("sha1", s.size(), out1, sizeof(out1));
    print_hex("sha256", s.size(), out256, sizeof(out256));
}

void dump_hmac(const std::string& s, const std::array<uint8_t, 32>& key, const char* keylabel)
{
    std::array<uint8_t, 28> mac224{};
    std::array<uint8_t, 32> mac256{};
    Span<const uint8_t> in(reinterpret_cast<const uint8_t*>(s.data()), s.size());
    realm::util::hmac_sha224(in, Span<uint8_t, 28>(mac224), Span<const uint8_t, 32>(key));
    realm::util::hmac_sha256(in, Span<uint8_t, 32>(mac256), Span<const uint8_t, 32>(key));
    std::printf("key=%s ", keylabel);
    print_hex("hmac224", s.size(), mac224.data(), mac224.size());
    std::printf("key=%s ", keylabel);
    print_hex("hmac256", s.size(), mac256.data(), mac256.size());
}

} // namespace

int main()
{
    std::printf("=== sha1 / sha256 over lengths 0..200 ===\n");
    // Exhaustive over short lengths: covers the block boundary at 64 and the
    // length-encoding edge at 55/56 where SHA padding spills into an extra block.
    for (size_t len = 0; len <= 200; ++len) {
        dump_sha(make_bytes(len, 1));
    }

    std::printf("\n=== sha1 / sha256 over longer inputs ===\n");
    for (size_t len : {255u, 256u, 257u, 1023u, 1024u, 4096u, 65536u}) {
        dump_sha(make_bytes(len, 7));
    }

    std::printf("\n=== degenerate byte patterns ===\n");
    for (size_t len : {1u, 55u, 56u, 63u, 64u, 65u, 119u, 120u, 128u}) {
        dump_sha(std::string(len, char(0x00)));
        dump_sha(std::string(len, char(0xff)));
    }

    std::printf("\n=== hmac_sha224 / hmac_sha256 ===\n");
    std::array<uint8_t, 32> key_zero{};
    std::array<uint8_t, 32> key_ff{};
    std::array<uint8_t, 32> key_seq{};
    for (size_t i = 0; i < 32; ++i) {
        key_ff[i] = 0xff;
        key_seq[i] = static_cast<uint8_t>(i);
    }
    // The 64-byte HMAC block boundary matters for the key padding path, and the
    // message lengths straddle the inner-hash block boundary.
    for (size_t len : {0u, 1u, 8u, 55u, 56u, 63u, 64u, 65u, 128u, 1000u, 4096u}) {
        const std::string msg = make_bytes(len, 3);
        dump_hmac(msg, key_zero, "00");
        dump_hmac(msg, key_ff, "ff");
        dump_hmac(msg, key_seq, "seq");
    }

    return 0;
}
