// Differential driver for upstream/src/realm/util/base64.cpp.
//
// Why this exists
// ---------------
// `make diff-test` cannot judge this unit. base64 never reaches a `.realm` file —
// every call site (to_json, uuid, serializer, bson, and the disabled sync/app code)
// treats the result as text. A wrong base64 port and a right one produce the same
// bytes on disk, so the project's normal gate would go green either way.
//
// So this driver is compiled twice against the *same* source, once linked to the
// oracle's C++ object and once to the Rust staticlib, and the two dumps are
// byte-compared. That restores the property diff-test provides everywhere else:
// two independent implementations, one input set, identical output required.
//
// Build and run via migration/checks/run_base64_differential.sh.
//
// The driver deliberately calls through the real <realm/util/base64.hpp>, so the
// Rust side is exercised through the genuine C++ types — Span, std::optional<size_t>
// and std::optional<std::vector<char>> — rather than through a hand-rolled
// declaration that could be wrong in the same way on both sides.

#include <realm/util/base64.hpp>

#include <cstdint>
#include <cstdio>
#include <string>
#include <vector>

namespace {

// A fixed, seeded PRNG. std::mt19937 would do, but an explicit one guarantees the
// two builds see identical input even if they were compiled by different toolchains.
struct Rng {
    std::uint64_t s = 0x9E3779B97F4A7C15ull;
    std::uint64_t next()
    {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        return s;
    }
    std::uint8_t byte()
    {
        return static_cast<std::uint8_t>(next() >> 33);
    }
};

void emit_bytes(const char* label, const char* data, size_t size)
{
    std::printf("%s[%zu]=", label, size);
    for (size_t i = 0; i < size; ++i)
        std::printf("%02x", static_cast<unsigned char>(data[i]));
    std::printf("\n");
}

// One encode case. The output buffer is deliberately oversized and pre-filled with a
// sentinel, and the *whole* buffer is dumped: that catches a port that writes the
// right answer plus one extra byte, or that leaves a byte of the padding untouched.
void probe_encode(const std::string& input)
{
    size_t needed = realm::util::base64_encoded_size(input.size());
    std::vector<char> out(needed + 8, '\xAB');
    size_t n = realm::util::base64_encode(input, realm::util::Span<char>(out.data(), out.size()));
    std::printf("encode n=%zu ", n);
    emit_bytes("out", out.data(), out.size());
}

void probe_decode(const std::string& input)
{
    size_t needed = realm::util::base64_decoded_size(input.size());
    std::vector<char> out(needed + 8, '\xAB');
    auto n = realm::util::base64_decode(input, realm::util::Span<char>(out.data(), out.size()));
    if (!n) {
        std::printf("decode none\n");
        return;
    }
    std::printf("decode n=%zu ", *n);
    emit_bytes("out", out.data(), out.size());
}

// This is the one that exercises the ABI rather than the algorithm: the result is a
// std::optional<std::vector<char>> returned via sret, holding a buffer that this
// (C++) frame will free. If the Rust got the vector layout, the capacity slot, or the
// allocator wrong, it shows up here — as a wrong size, a wrong capacity, or a crash
// in the destructor.
void probe_decode_to_vector(const std::string& input)
{
    auto v = realm::util::base64_decode_to_vector(input);
    if (!v) {
        std::printf("to_vector none\n");
        return;
    }
    std::printf("to_vector size=%zu cap=%zu empty=%d ", v->size(), v->capacity(), int(v->empty()));
    emit_bytes("data", v->data() ? v->data() : "", v->size());
    // Touch every byte through the returned object so a bogus end pointer is fatal
    // here rather than silently accepted.
    std::uint32_t sum = 0;
    for (char c : *v)
        sum = sum * 31 + static_cast<unsigned char>(c);
    std::printf("to_vector checksum=%u\n", sum);
    // v is destroyed here: operator delete on a Rust-produced pointer.
}

void probe_all(const std::string& s)
{
    probe_encode(s);
    probe_decode(s);
    probe_decode_to_vector(s);
}

} // namespace

int main()
{
    // 1. Every single byte, encoded and decoded. Covers all 256 decode-table entries,
    //    including the whitespace and URL-safe-alphabet quirks.
    for (int b = 0; b < 256; ++b)
        probe_all(std::string(1, static_cast<char>(b)));

    // 2. All strings of length 0..3 over an alphabet chosen to mix valid base64
    //    characters, padding, whitespace (tab/LF/CR/space — CR is invalid in this
    //    table and must stay that way) and outright invalid bytes.
    static const char alphabet[] = {'A', 'z', '0', '+', '/', '-', '_', '=', ' ', '\t', '\n', '\r', '*', '\0'};
    constexpr int na = int(sizeof(alphabet));
    for (int a = 0; a < na; ++a) {
        probe_all(std::string(1, alphabet[a]));
        for (int b = 0; b < na; ++b) {
            probe_all(std::string({alphabet[a], alphabet[b]}));
            for (int c = 0; c < na; ++c) {
                probe_all(std::string({alphabet[a], alphabet[b], alphabet[c]}));
            }
        }
    }

    // 3. Round-trip: random binary of every length 0..=257, so each residue of the
    //    3-byte encode group and the 4-char decode group is hit many times, well past
    //    any plausible off-by-one at a buffer boundary.
    Rng rng;
    for (size_t len = 0; len <= 257; ++len) {
        std::string data;
        data.reserve(len);
        for (size_t i = 0; i < len; ++i)
            data.push_back(static_cast<char>(rng.byte()));

        std::string encoded(realm::util::base64_encoded_size(data.size()), '\0');
        size_t n = realm::util::base64_encode(data, encoded);
        encoded.resize(n);
        std::printf("rt len=%zu enc=%s\n", len, encoded.c_str());
        probe_all(encoded);

        // And the same encoded text with whitespace injected, which is the case the
        // "extra = input.size() % 4" fallback gets wrong if it is re-derived rather
        // than mirrored: that line counts *all* characters, whitespace included.
        std::string spaced;
        for (size_t i = 0; i < encoded.size(); ++i) {
            spaced.push_back(encoded[i]);
            if (i % 3 == 1)
                spaced.push_back(' ');
            if (i % 7 == 2)
                spaced.push_back('\n');
        }
        probe_all(spaced);

        // Unpadded, which exercises the inferred-padding path.
        std::string unpadded = encoded;
        while (!unpadded.empty() && unpadded.back() == '=')
            unpadded.pop_back();
        probe_all(unpadded);
    }

    // 4. The overrun family: 4k valid characters followed by exactly one '='.
    //
    //    base64_decode reports 3k+2 bytes for these while base64_decoded_size only
    //    reserves 3k+1, so base64_decode_to_vector's resize *grows* and reallocates.
    //    That is the one case where the vector's capacity is not max_size, and the
    //    first version of this port got it wrong — it is here because the check
    //    caught it, and it stays here so a later refactor cannot un-catch it.
    //
    //    Whitespace variants are included to confirm the opposite: padding the input
    //    with ignored characters inflates the reservation and removes the overrun,
    //    so those must come back with capacity == max_size.
    static const char valid[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for (int k = 0; k <= 8; ++k) {
        for (int shift = 0; shift < 5; ++shift) {
            std::string s;
            for (int i = 0; i < 4 * k; ++i)
                s.push_back(valid[(i * 7 + shift * 13) % 64]);
            probe_all(s + "=");
            probe_all(s + "==");
            probe_all(s + " =");
            probe_all(s + "=\n");
            probe_all(s + "\t=");
        }
    }

    // 5. Long random *decode* inputs over the raw byte range, so invalid bytes appear
    //    at every position rather than only at the ends.
    for (size_t len = 0; len <= 64; ++len) {
        std::string junk;
        for (size_t i = 0; i < len; ++i)
            junk.push_back(static_cast<char>(rng.byte()));
        probe_decode(junk);
        probe_decode_to_vector(junk);
    }

    std::printf("done\n");
    return 0;
}
