// Differential driver for upstream/src/realm/error_codes.cpp.
//
// This unit is reachable but byte-invisible: error names and categories are
// diagnostics and never reach a `.realm`. `make verify` therefore cannot distinguish
// a correct port from a wrong one — it only proves the link is intact. This driver is
// the evidence.
//
// It matters more here than for most units, because the Rust port's two lookup tables
// were *generated from the oracle* rather than re-typed from the C++ (see
// crates/realm-core-rs/src/error_codes_table.rs). That makes transcription error
// impossible and makes a stale or truncated generation the live risk instead — which
// is exactly what a row-by-row comparison against the oracle catches.
//
// Build and run via migration/checks/run_error_codes_differential.sh.

#include <realm/error_codes.hpp>

#include <cstdio>
#include <string>
#include <string_view>
#include <vector>

using namespace realm;

namespace {

void print_sv(std::string_view sv)
{
    // Print length explicitly: a string_view with the right bytes and the wrong size
    // would otherwise look identical.
    std::printf("[%zu]%.*s", sv.size(), int(sv.size()), sv.data());
}

} // namespace

int main()
{
    // 1. The whole table, in order, with the code and the category bits. Order is
    //    load-bearing: from_string binary-searches it, and error_string returns the
    //    first code match.
    auto list = ErrorCodes::get_error_list();
    std::printf("error_list size=%zu cap=%zu\n", list.size(), list.capacity());
    for (size_t i = 0; i < list.size(); ++i) {
        std::printf("row %3zu code=%d cat=%u name=", i, int(list[i].second),
                    ErrorCodes::error_categories(list[i].second).value());
        print_sv(list[i].first);
        std::printf("\n");
    }

    // 2. get_all_codes / get_all_names must agree element-for-element with the list,
    //    and their capacities are observable on the returned vectors.
    auto codes = ErrorCodes::get_all_codes();
    auto names = ErrorCodes::get_all_names();
    std::printf("all_codes size=%zu cap=%zu\n", codes.size(), codes.capacity());
    std::printf("all_names size=%zu cap=%zu\n", names.size(), names.capacity());
    for (size_t i = 0; i < codes.size(); ++i)
        std::printf("code %3zu = %d\n", i, int(codes[i]));
    for (size_t i = 0; i < names.size(); ++i) {
        std::printf("name %3zu = ", i);
        print_sv(names[i]);
        std::printf("\n");
    }

    // 3. error_string and error_categories over every table code plus a wide sweep of
    //    non-table values, including the two codes that have categories but no name,
    //    negatives, and the int extremes.
    for (auto& [name, code] : list) {
        std::printf("str(%d)=", int(code));
        print_sv(ErrorCodes::error_string(code));
        std::printf(" cat=%u\n", ErrorCodes::error_categories(code).value());
    }
    static const long sweep[] = {-2147483648L, -1000, -2, -1, 0, 1, 2, 1044, 1045, 1046, 1047,
                                 1999999, 2000000, 2000001, 30000, 65536, 999999, 1000001,
                                 4000000, 2147483647L};
    for (long v : sweep) {
        auto code = ErrorCodes::Error(v);
        std::printf("sweep(%ld) str=", v);
        print_sv(ErrorCodes::error_string(code));
        std::printf(" cat=%u\n", ErrorCodes::error_categories(code).value());
    }

    // 4. from_string. The comparator is strncmp over the NEEDLE's length, so prefixes
    //    of a table entry compare equal during the search and are rejected only by the
    //    exact equality check afterwards. Every one of those cases is probed:
    //    exact hits, every proper prefix of a few entries, entries with a suffix
    //    appended, case changes, and the empty string.
    for (auto& [name, code] : list) {
        std::string s(name);
        std::printf("from(%s)=%d\n", s.c_str(), int(ErrorCodes::from_string(s)));
        // every proper prefix
        for (size_t k = 0; k < s.size(); ++k)
            std::printf("  pre(%zu)=%d\n", k, int(ErrorCodes::from_string(std::string_view(s.data(), k))));
        // one byte appended, and one byte changed at the end
        std::string longer = s + "X";
        std::printf("  ext=%d\n", int(ErrorCodes::from_string(longer)));
        std::string altered = s;
        altered.back() = char(altered.back() ^ 0x20);
        std::printf("  alt(%s)=%d\n", altered.c_str(), int(ErrorCodes::from_string(altered)));
    }

    // 5. Needles that are not near any entry, plus embedded NUL, which strncmp treats
    //    as a terminator while string_view equality does not.
    static const char* misses[] = {"", "A", "a", "zzzzzz", "OK ", " OK", "\x01", "~"};
    for (const char* m : misses)
        std::printf("miss(%s)=%d\n", m, int(ErrorCodes::from_string(m)));
    {
        const char embedded[] = {'O', 'K', '\0', 'X'};
        std::printf("embedded_nul=%d\n", int(ErrorCodes::from_string(std::string_view(embedded, 4))));
        std::printf("embedded_nul2=%d\n", int(ErrorCodes::from_string(std::string_view(embedded, 3))));
    }

    std::printf("done\n");
    return 0;
}
