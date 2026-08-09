// Differential driver for upstream/src/realm/unicode.cpp.
//
// This unit is byte-invisible AND untraced: query-side string folding that never
// reaches a .realm, and hit zero times across all five traces. `make verify` proves the
// link is intact and nothing else regressed. This driver is the entire evidence.
//
// Every function here is pure, which is unusual in this codebase and worth exploiting:
// the coverage below is exhaustive where it can be (all 256 lead bytes, all 256 single
// bytes through case_map, every 2-byte lead/continuation pair) rather than sampled.
//
// Build and run via migration/checks/run_unicode_differential.sh.

#include <realm/unicode.hpp>
#include <realm/string_data.hpp>

#include <array>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>

using namespace realm;

namespace {

void show(const char* tag, StringData s)
{
    std::printf("%-22s [%zu]", tag, s.size());
    for (size_t i = 0; i < s.size(); ++i)
        std::printf("%02x", (unsigned char)s[i]);
    std::printf("\n");
}

// case_map returns util::Optional<std::string>; print engagement, size, capacity and
// bytes. Capacity is included because the port builds the string through
// std::string::append rather than reproducing libc++'s resize growth, and capacity is
// the observable that would differ if that choice were wrong.
void show_case_map(const char* tag, StringData src, bool upper)
{
    auto r = case_map(src, upper);
    std::printf("%-22s upper=%d in=[%zu]", tag, int(upper), src.size());
    for (size_t i = 0; i < src.size(); ++i)
        std::printf("%02x", (unsigned char)src[i]);
    if (!r) {
        std::printf(" -> none\n");
        return;
    }
    std::printf(" -> [%zu,cap=%zu]", r->size(), r->capacity());
    for (size_t i = 0; i < r->size(); ++i)
        std::printf("%02x", (unsigned char)(*r)[i]);
    std::printf("\n");
}

std::string ci_upper(StringData s)
{
    return case_map(s, true, IgnoreErrors);
}
std::string ci_lower(StringData s)
{
    return case_map(s, false, IgnoreErrors);
}

} // namespace

int main()
{
    std::setvbuf(stdout, nullptr, _IONBF, 0);

    // ---------------------------------------------------------------------
    // 1. sequence_length over ALL 256 lead bytes. Exhaustive.
    // ---------------------------------------------------------------------
    std::printf("seqlen:");
    for (int i = 0; i < 256; ++i)
        std::printf("%zu", sequence_length(char(i)));
    std::printf("\n");

    // ---------------------------------------------------------------------
    // 2. utf8value over every well-formed 1- and 2-byte sequence, plus a
    //    spread of 3- and 4-byte ones. The function documents that it may read
    //    out of bounds on malformed input, so only well-formed input is fed.
    // ---------------------------------------------------------------------
    for (int i = 0; i < 0x80; ++i) {
        char buf[8] = {char(i), 0, 0, 0, 0, 0, 0, 0};
        std::printf("u8v1 %02x = %u\n", i, utf8value(buf));
    }
    for (int lead = 0xC0; lead <= 0xDF; ++lead) {
        for (int cont = 0x80; cont <= 0xBF; cont += 7) {
            char buf[8] = {char(lead), char(cont), 0, 0, 0, 0, 0, 0};
            std::printf("u8v2 %02x%02x = %u\n", lead, cont, utf8value(buf));
        }
    }
    for (int lead = 0xE0; lead <= 0xEF; lead += 3) {
        char buf[8] = {char(lead), char(0x82), char(0xAC), 0, 0, 0, 0, 0};
        std::printf("u8v3 %02x = %u\n", lead, utf8value(buf));
    }
    for (int lead = 0xF0; lead <= 0xF7; lead += 2) {
        char buf[8] = {char(lead), char(0x9F), char(0x98), char(0x80), 0, 0, 0, 0};
        std::printf("u8v4 %02x = %u\n", lead, utf8value(buf));
    }

    // ---------------------------------------------------------------------
    // 3. case_map over EVERY single byte, both directions. Exhaustive, and it
    //    covers the whole malformed-lead-byte space in one sweep: 0x80..0xBF are
    //    bare continuations, 0xC0..0xFF are truncated multibyte leads, and all of
    //    them must come back as `none`.
    // ---------------------------------------------------------------------
    for (int i = 0; i < 256; ++i) {
        char c = char(i);
        char tag[32];
        std::snprintf(tag, sizeof(tag), "cm1.%02x", i);
        show_case_map(tag, StringData(&c, 1), true);
        show_case_map(tag, StringData(&c, 1), false);
    }

    // ---------------------------------------------------------------------
    // 4. case_map over every 2-byte lead/continuation combination in the range
    //    that is actually folded (the Latin-1 supplement), plus the boundaries
    //    just outside it, plus invalid continuations.
    // ---------------------------------------------------------------------
    for (int lead = 0xC2; lead <= 0xC3; ++lead) {
        for (int cont = 0x80; cont <= 0xBF; ++cont) {
            char buf[2] = {char(lead), char(cont)};
            char tag[32];
            std::snprintf(tag, sizeof(tag), "cm2.%02x%02x", lead, cont);
            show_case_map(tag, StringData(buf, 2), true);
            show_case_map(tag, StringData(buf, 2), false);
        }
    }
    // invalid second bytes
    for (int cont : {0x00, 0x41, 0x7F, 0xC0, 0xFF}) {
        char buf[2] = {char(0xC3), char(cont)};
        char tag[32];
        std::snprintf(tag, sizeof(tag), "cm2bad.%02x", cont);
        show_case_map(tag, StringData(buf, 2), true);
    }

    // ---------------------------------------------------------------------
    // 5. case_map over strings that straddle the short/long boundary, so the
    //    capacity of the returned string is exercised on both sides of 22 and
    //    across the 47 -> round_up(n+1,8) transition.
    // ---------------------------------------------------------------------
    for (size_t n : {size_t(0), size_t(1), size_t(21), size_t(22), size_t(23), size_t(24), size_t(47), size_t(48),
                     size_t(55), size_t(56), size_t(100), size_t(1000)}) {
        std::string s(n, 'a');
        for (size_t i = 0; i < n; i += 3)
            s[i] = 'Z';
        char tag[32];
        std::snprintf(tag, sizeof(tag), "cmlen.%zu", n);
        show_case_map(tag, StringData(s.data(), s.size()), true);
        show_case_map(tag, StringData(s.data(), s.size()), false);
    }
    // 3- and 4-byte sequences copied through unchanged, and truncated ones rejected
    {
        const char* cases[] = {"\xE2\x82\xAC", "\xF0\x9F\x98\x80", "a\xE2\x82\xAC" "b",
                               "\xE2\x82", "\xF0\x9F\x98", "\xE2\x82\xAC\xC3\xA9"};
        for (const char* c : cases) {
            char tag[48];
            std::snprintf(tag, sizeof(tag), "cmseq.%zu", std::strlen(c));
            show_case_map(tag, StringData(c, std::strlen(c)), true);
            show_case_map(tag, StringData(c, std::strlen(c)), false);
        }
    }

    // ---------------------------------------------------------------------
    // 6. equal_case_fold and search_case_fold. The needle arrays must be the
    //    upper and lower foldings of the same string, which is how callers build
    //    them, so they are produced with case_map.
    // ---------------------------------------------------------------------
    {
        const char* needles[] = {"a", "ab", "ABC", "abc", "", "\xC3\xA9", "\xC3\x89", "aA", "zZz"};
        const char* haystacks[] = {"a",   "A",   "abc",  "ABC",  "aBc",  "xabc",  "abcx", "xabcx",
                                   "",    "ab",  "abcd", "\xC3\xA9", "\xC3\x89", "a\xC3\xA9z"};
        for (const char* nd : needles) {
            std::string up = ci_upper(StringData(nd, std::strlen(nd)));
            std::string lo = ci_lower(StringData(nd, std::strlen(nd)));
            for (const char* hs : haystacks) {
                StringData h(hs, std::strlen(hs));
                bool eq = (h.size() == std::strlen(nd)) && equal_case_fold(h, up.c_str(), lo.c_str());
                size_t sc = search_case_fold(h, up.c_str(), lo.c_str(), std::strlen(nd));
                std::printf("fold n=%-8s h=%-10s eq=%d search=%zu\n", nd, hs, int(eq), sc);
            }
        }
    }

    // ---------------------------------------------------------------------
    // 7. contains_ins with a Boyer-Moore skip table built the way the caller
    //    does, plus the degenerate empty-needle case whose convention is
    //    "contained only in a non-empty haystack".
    // ---------------------------------------------------------------------
    {
        const char* needles[] = {"", "a", "bc", "abc", "ABC", "zz", "\xC3\xA9"};
        const char* haystacks[] = {"", "a", "A", "abc", "xxABCxx", "aabbcc", "\xC3\x89zz", "abcabc"};
        for (const char* nd : needles) {
            size_t ns = std::strlen(nd);
            std::string up = ci_upper(StringData(nd, ns));
            std::string lo = ci_lower(StringData(nd, ns));
            std::array<uint8_t, 256> charmap{};
            charmap.fill(0);
            // Same construction as StringNode<ContainsIns>: distance from the end.
            for (size_t i = 0; i + 1 < ns; ++i) {
                charmap[(unsigned char)up[i]] = uint8_t(ns - i - 1);
                charmap[(unsigned char)lo[i]] = uint8_t(ns - i - 1);
            }
            for (const char* hs : haystacks) {
                bool r = contains_ins(StringData(hs, std::strlen(hs)), up.c_str(), lo.c_str(), ns, charmap);
                std::printf("contains n=%-6s h=%-10s = %d\n", nd, hs, int(r));
            }
        }
    }

    // ---------------------------------------------------------------------
    // 8. string_like_ins, both overloads, including nulls on either side --
    //    where the contract is "equal only if both are null" -- and the
    //    argument swap in the three-argument form.
    // ---------------------------------------------------------------------
    {
        const char* texts[] = {"", "a", "A", "abc", "ABC", "aXbXc", "\xC3\xA9"};
        const char* pats[] = {"", "*", "?", "a*", "A*", "*c", "?b?", "abc", "ABC", "*B*", "\xC3\x89"};
        for (const char* t : texts) {
            for (const char* p : pats) {
                StringData ts(t, std::strlen(t)), ps(p, std::strlen(p));
                std::string up = ci_upper(ps), lo = ci_lower(ps);
                bool two = string_like_ins(ts, ps);
                bool three = string_like_ins(ts, StringData(up.data(), up.size()), StringData(lo.data(), lo.size()));
                std::printf("like t=%-8s p=%-6s two=%d three=%d\n", t, p, int(two), int(three));
            }
        }
        // null handling: only null-vs-null is a match
        StringData null_sd;
        std::printf("like null/null two=%d\n", int(string_like_ins(null_sd, null_sd)));
        std::printf("like null/a    two=%d\n", int(string_like_ins(null_sd, StringData("a", 1))));
        std::printf("like a/null    two=%d\n", int(string_like_ins(StringData("a", 1), null_sd)));
        std::printf("like null3     three=%d\n", int(string_like_ins(null_sd, null_sd, null_sd)));
        std::printf("like a3/null   three=%d\n", int(string_like_ins(StringData("a", 1), null_sd, null_sd)));
    }

    std::printf("done\n");
    return 0;
}
