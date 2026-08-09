// Differential driver for upstream/src/realm/decimal128.cpp -- the methods.
//
// `migration/checks/decimal128/run_conv_differential.sh` covers the other half, the
// reimplemented realm_binary64_to_bid128, over 3M doubles at every rounding mode. This
// covers the 47 symbols that are wrappers over linkable BID entry points, plus the
// Decimal128(double) constructor end to end (conversion *and* the two quantize retries,
// which the conv differential does not reach).
//
// This unit is byte-visible and UNTRACED: Decimal128 is a stored column type, but the
// trace schema is int/string/double/bool, so no trace stores a decimal. `make verify`
// proves the link is intact and nothing else regressed. This driver is the evidence.
//
// What it has to print, and why:
//
//   * the raw 16 bytes of every Decimal128. Decimal128 has REDUNDANT REPRESENTATIONS --
//     the same number at different coefficient/exponent pairings -- so comparing values
//     would pass on a port that stores 1E+1 where the C++ stores 10E+0. That is exactly
//     the divergence class this project gates on, and it is invisible to ==.
//   * std::string CAPACITY as well as contents, for to_string(). The base64 port shipped
//     a wrong std::vector capacity that no printed byte revealed.
//   * the engaged flag of the optionals, separately from the value.
//
// Build and run via migration/checks/run_decimal128_differential.sh.

#include <realm/decimal128.hpp>
#include <realm/string_data.hpp>

#include <cstdint>
#include <cstdio>
#include <cstring>
#include <limits>

using namespace realm;

namespace {

void show(const char* tag, const Decimal128& d)
{
    const auto* raw = d.raw();
    std::printf("%-30s %016llx %016llx null=%d nan=%d\n", tag, (unsigned long long)raw->w[0],
                (unsigned long long)raw->w[1], int(d.is_null()), int(d.is_nan()));
}

void show_string(const char* tag, const Decimal128& d)
{
    std::string s = d.to_string();
    std::printf("%-30s [%zu,cap=%zu] %s\n", tag, s.size(), s.capacity(), s.c_str());
}

void show_bids(const char* tag, const Decimal128& d)
{
    auto b32 = d.to_bid32();
    auto b64 = d.to_bid64();
    std::printf("%-30s b32=%d/%08x b64=%d/%016llx\n", tag, int(bool(b32)), b32 ? b32->u : 0u,
                int(bool(b64)), b64 ? (unsigned long long)b64->w : 0ull);
}

void show_unpack(const char* tag, const Decimal128& d)
{
    Decimal128::Bid128 coeff;
    int exponent;
    bool sign;
    d.unpack(coeff, exponent, sign);
    int64_t i = 0;
    bool ok = d.to_int(i);
    std::printf("%-30s coeff=%016llx%016llx exp=%d sign=%d to_int=%d/%lld\n", tag,
                (unsigned long long)coeff.w[1], (unsigned long long)coeff.w[0], exponent, int(sign), int(ok),
                (long long)i);
}

// Every comparator over one pair, printed together so a single wrong branch shows up as
// one changed column.
void compare_all(const Decimal128& a, const Decimal128& b)
{
    std::printf("cmp %016llx%016llx vs %016llx%016llx  eq=%d ne=%d lt=%d gt=%d le=%d ge=%d c=%d\n",
                (unsigned long long)a.raw()->w[1], (unsigned long long)a.raw()->w[0],
                (unsigned long long)b.raw()->w[1], (unsigned long long)b.raw()->w[0], int(a == b), int(a != b),
                int(a < b), int(a > b), int(a <= b), int(a >= b), a.compare(b));
}

const char* const kStrings[] = {
    "0",      "1",       "-1",      "0.1",     "-0.1",   "1E+10",   "1E-10",   "123456789012345678901234567890123",
    "1234567890123456789012345678901234",      "-1234567890123456789012345678901234",
    "9.999999999999999999999999999999999E+6144", "1E-6176", "Inf",   "-Inf",    "NaN",    "-NaN",
    "SNaN",   "0.000",   "10",      "100E-2",  "1.50",    "1.5",     "",        "garbage",
};

const double kDoubles[] = {
    0.0,   -0.0,  1.0,   -1.0,   0.5,    -0.25,  3.14159265358979,  2.718281828459045,
    1e-300, 1e300, 5e-324, 1.7976931348623157e308, 0.1, 0.2, 0.3, 1.0 / 3.0,
    123456789.0, 1e15, 1e16, 9007199254740993.0, -1e-15,
    std::numeric_limits<double>::infinity(), -std::numeric_limits<double>::infinity(),
    std::numeric_limits<double>::quiet_NaN(),
};

const int64_t kInts[] = {
    0, 1, -1, 2, -2, 10, -10, 999999999999999LL, -999999999999999LL,
    9223372036854775807LL, -9223372036854775807LL - 1, 1000000, -1000000,
};

} // namespace

int main()
{
    std::setvbuf(stdout, nullptr, _IONBF, 0);

    // ---------------------------------------------------------------------
    // 1. Constructors. Every one of the eleven, including both RoundTo modes
    //    and the float overload that forwards to Digits7.
    // ---------------------------------------------------------------------
    show("default", Decimal128());
    show("null", Decimal128(realm::null()));
    for (int64_t v : kInts) {
        char tag[48];
        std::snprintf(tag, sizeof(tag), "int64(%lld)", (long long)v);
        show(tag, Decimal128(v));
    }
    for (int64_t v : kInts) {
        char tag[48];
        std::snprintf(tag, sizeof(tag), "uint64(%llu)", (unsigned long long)v);
        show(tag, Decimal128(uint64_t(v)));
    }
    for (int v : {0, 1, -1, 2147483647, -2147483647 - 1}) {
        char tag[48];
        std::snprintf(tag, sizeof(tag), "int(%d)", v);
        show(tag, Decimal128(v));
    }
    for (double v : kDoubles) {
        char tag[64];
        std::snprintf(tag, sizeof(tag), "double15(%.17g)", v);
        show(tag, Decimal128(v, Decimal128::RoundTo::Digits15));
        std::snprintf(tag, sizeof(tag), "double7(%.17g)", v);
        show(tag, Decimal128(v, Decimal128::RoundTo::Digits7));
        std::snprintf(tag, sizeof(tag), "float(%.17g)", v);
        show(tag, Decimal128(float(v)));
    }
    for (const char* s : kStrings) {
        char tag[64];
        std::snprintf(tag, sizeof(tag), "str(%s)", s);
        show(tag, Decimal128(StringData(s, std::strlen(s))));
        std::printf("%-30s valid=%d\n", tag, int(Decimal128::is_valid_str(StringData(s, std::strlen(s)))));
    }
    // Bid128 + exponent + sign, across the exponent range and both signs.
    for (int exp : {0, 1, -1, 100, -100, 6111, -6176, 6144}) {
        for (bool sign : {false, true}) {
            Decimal128::Bid128 coeff{{1234567890ull, 0ull}};
            char tag[64];
            std::snprintf(tag, sizeof(tag), "bid128(exp=%d,s=%d)", exp, int(sign));
            show(tag, Decimal128(coeff, exp, sign));
        }
    }
    // Bid32 / Bid64, including the two NULL sentinels that short-circuit.
    for (uint32_t u : {0u, 1u, 0x7c0000aau, 0x32800001u, 0xf8000000u}) {
        char tag[48];
        std::snprintf(tag, sizeof(tag), "bid32(%08x)", u);
        show(tag, Decimal128(Decimal128::Bid32(u)));
    }
    for (uint64_t w : {0ull, 1ull, 0x7c000000000000aaull, 0x31c0000000000001ull}) {
        char tag[48];
        std::snprintf(tag, sizeof(tag), "bid64(%016llx)", (unsigned long long)w);
        show(tag, Decimal128(Decimal128::Bid64(w)));
    }
    for (const char* n : {"0", "1", "123", "-1", "999999"}) {
        char tag[48];
        std::snprintf(tag, sizeof(tag), "nan(%s)", n);
        show(tag, Decimal128::nan(n));
    }

    // ---------------------------------------------------------------------
    // 2. to_string, to_bid32/64, unpack, to_int. to_string has two distinct
    //    paths -- the bid128_to_string one when the coefficient needs the high
    //    word, and the hand-rolled one when it does not -- so both are covered.
    // ---------------------------------------------------------------------
    {
        const Decimal128 vals[] = {
            Decimal128(),
            Decimal128(realm::null()),
            Decimal128(int64_t(0)),
            Decimal128(int64_t(1)),
            Decimal128(int64_t(-1)),
            Decimal128(StringData("0.1", 3)),
            Decimal128(StringData("1E+10", 5)),
            Decimal128(StringData("1E-10", 5)),
            Decimal128(StringData("1234567890123456789012345678901234", 34)),
            Decimal128(StringData("-1234567890123456789012345678901234", 35)),
            Decimal128(StringData("Inf", 3)),
            Decimal128(StringData("-Inf", 4)),
            Decimal128(StringData("NaN", 3)),
            Decimal128(1.0 / 3.0, Decimal128::RoundTo::Digits15),
            Decimal128(1e300, Decimal128::RoundTo::Digits15),
            Decimal128(5e-324, Decimal128::RoundTo::Digits15),
            Decimal128::nan("42"),
        };
        int i = 0;
        for (const auto& d : vals) {
            char tag[48];
            std::snprintf(tag, sizeof(tag), "tostr[%d]", i);
            show_string(tag, d);
            std::snprintf(tag, sizeof(tag), "bids[%d]", i);
            show_bids(tag, d);
            std::snprintf(tag, sizeof(tag), "unpack[%d]", i);
            show_unpack(tag, d);
            ++i;
        }
        // Long strings, to push to_string's result out of the SSO buffer and make the
        // capacity column say something.
        for (int digits = 1; digits <= 34; ++digits) {
            std::string s(size_t(digits), '9');
            char tag[48];
            std::snprintf(tag, sizeof(tag), "tostr.len%d", digits);
            show_string(tag, Decimal128(StringData(s.data(), s.size())));
        }
        for (int e : {-40, -20, -5, -1, 0, 1, 5, 20, 40}) {
            char buf[64];
            std::snprintf(buf, sizeof(buf), "1234567890E%+d", e);
            char tag[48];
            std::snprintf(tag, sizeof(tag), "tostr.exp%+d", e);
            show_string(tag, Decimal128(StringData(buf, std::strlen(buf))));
        }
    }

    // ---------------------------------------------------------------------
    // 3. Comparisons, including null-vs-null, NaN-vs-NaN (which must be a
    //    stable order, not just "false"), and NaN-vs-number.
    // ---------------------------------------------------------------------
    {
        const Decimal128 vals[] = {
            Decimal128(int64_t(0)),  Decimal128(int64_t(1)),   Decimal128(int64_t(-1)),
            Decimal128(realm::null()), Decimal128(StringData("NaN", 3)),
            Decimal128(StringData("-NaN", 4)), Decimal128(StringData("Inf", 3)),
            Decimal128(StringData("-Inf", 4)), Decimal128(StringData("1.0", 3)),
            Decimal128(StringData("10E-1", 5)), Decimal128::nan("1"), Decimal128::nan("2"),
        };
        for (const auto& a : vals)
            for (const auto& b : vals)
                compare_all(a, b);
    }

    // ---------------------------------------------------------------------
    // 4. Arithmetic. Every overload of * and /, plus += and -=. Results are
    //    printed raw, because 1E+1 and 10E+0 are the same number and different
    //    bytes.
    // ---------------------------------------------------------------------
    {
        const Decimal128 lhs[] = {
            Decimal128(int64_t(1)), Decimal128(int64_t(-7)), Decimal128(StringData("0.1", 3)),
            Decimal128(StringData("1E+20", 5)), Decimal128(StringData("NaN", 3)),
            Decimal128(realm::null()),
        };
        for (const auto& a : lhs) {
            for (int64_t m : {int64_t(0), int64_t(1), int64_t(-1), int64_t(3), int64_t(1000000)}) {
                char tag[64];
                std::snprintf(tag, sizeof(tag), "mul.i64(%lld)", (long long)m);
                show(tag, a * m);
                std::snprintf(tag, sizeof(tag), "div.i64(%lld)", (long long)m);
                show(tag, a / m);
            }
            for (int m : {0, 1, -1, 7}) {
                char tag[64];
                std::snprintf(tag, sizeof(tag), "mul.int(%d)", m);
                show(tag, a * m);
                std::snprintf(tag, sizeof(tag), "div.int(%d)", m);
                show(tag, a / m);
            }
            for (size_t m : {size_t(0), size_t(1), size_t(3), size_t(1000000)}) {
                char tag[64];
                std::snprintf(tag, sizeof(tag), "mul.size(%zu)", m);
                show(tag, a * m);
                std::snprintf(tag, sizeof(tag), "div.size(%zu)", m);
                show(tag, a / m);
            }
            for (const auto& b : lhs) {
                show("mul.dec", a * b);
                show("div.dec", a / b);
                Decimal128 t(a);
                t += b;
                show("plus.eq", t);
                Decimal128 u(a);
                u -= b;
                show("minus.eq", u);
                show("plus", a + b);
                show("minus", a - b);
            }
        }
    }

    // ---------------------------------------------------------------------
    // 5. operator==(Bid32, Bid32) -- a free function with its own hand-written
    //    comparison, including the exponent-difference cutoff at 6 and the
    //    coefficient overflow guard at 9999999.
    // ---------------------------------------------------------------------
    {
        const uint32_t xs[] = {
            0u, 1u, 0x32800001u, 0x32800002u, 0x32000064u, 0x31800001u,
            0x2f800001u, 0x33800001u, 0x7c0000aau, 0xf8000000u, 0x78000000u,
            0x32000000u, 0x6cb8967fu, 0x32989680u,
            // Pairs denoting the same number at exponents differing by exactly 5, 6 and
            // 7, plus significands that trip the 9999999 overflow guard mid-loop.
            // Without these the `exp_y - exp_x > 6` cutoff is never exercised: a control
            // that lowered it to 5 passed.
            0x190f4240u, 0x1c000001u, 0x190186a0u, 0x1b800001u, 0x19189680u, 0x1c800001u, 0x191e8480u, 0x19830d40u, 0x1998967fu, 0x198f423fu, 0x19000001u, 0x16189680u,
        };
        for (uint32_t x : xs)
            for (uint32_t y : xs)
                std::printf("bid32eq %08x %08x = %d\n", x, y,
                            int(Decimal128::Bid32(x) == Decimal128::Bid32(y)));
    }

    std::printf("done\n");
    return 0;
}
