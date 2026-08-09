// Differential driver for utilities.cpp.
//
// Compiled twice by run_utilities_differential.sh — once against the oracle's
// utilities.cpp.o, once against the Rust staticlib — and the two dumps must be
// byte-identical.
//
// What this actually tests, beyond the obvious:
//
//   1. the two exported *data* symbols. sse_support and avx_support are read by
//      cpu_sse<>()/cpu_avx<>() inlined into many other TUs; if the Rust definitions
//      differed, every one of those TUs would branch differently.
//   2. platform_timegm's deliberate 32-bit truncation, which is only visible for
//      timestamps past 2038-01-19.
//   3. FastRand::operator(), a member function whose `this` is a bare uint64_t.
//   4. fastrand's shared-state sequence, including the max == UINT64_MAX modulus.

#include <cstdio>
#include <cstdint>
#include <cstring>
#include <ctime>

#include <realm/utilities.hpp>

int main()
{
    std::printf("=== globals before cpuid_init ===\n");
    std::printf("sse_support=%d avx_support=%d\n", int(realm::sse_support), int(realm::avx_support));

    realm::cpuid_init();
    std::printf("=== globals after cpuid_init ===\n");
    std::printf("sse_support=%d avx_support=%d\n", int(realm::sse_support), int(realm::avx_support));

    std::printf("\n=== fast_popcount32 ===\n");
    const int32_t v32[] = {0,  1,  -1, 2,  -2, 3,  0x7fffffff, int32_t(0x80000000), int32_t(0xffffffff),
                           0x55555555, int32_t(0xaaaaaaaa), 0x0f0f0f0f, int32_t(0xf0f0f0f0), 123456789,
                           -123456789, 0xff, 0xff00, 0xff0000, int32_t(0xff000000)};
    for (int32_t v : v32)
        std::printf("popcount32(%11d) = %d\n", v, realm::fast_popcount32(v));

    std::printf("\n=== fast_popcount64 ===\n");
    const int64_t v64[] = {0,
                           1,
                           -1,
                           2,
                           -2,
                           int64_t(0x7fffffffffffffffLL),
                           int64_t(0x8000000000000000ULL),
                           int64_t(0x5555555555555555ULL),
                           int64_t(0xaaaaaaaaaaaaaaaaULL),
                           int64_t(0x00000000ffffffffULL),
                           int64_t(0xffffffff00000000ULL),
                           1234567890123456789LL,
                           -1234567890123456789LL};
    for (int64_t v : v64)
        std::printf("popcount64(%20lld) = %d\n", (long long)v, realm::fast_popcount64(v));

    std::printf("\n=== fastrand: seeded (pure function of the seed) ===\n");
    for (uint64_t seed : {0ULL, 1ULL, 2ULL, 42ULL, 1000ULL, 0xffffffffULL, 0xffffffffffffffffULL})
        std::printf("fastrand(seed=%20llu, is_seed=1) = %20llu\n", (unsigned long long)seed,
                    (unsigned long long)realm::fastrand(seed, true));

    std::printf("\n=== fastrand: shared-state sequence ===\n");
    // Both drivers start from the same static state (1) and make the same calls in the
    // same order, so the whole sequence must match element for element.
    for (int i = 0; i < 40; ++i) {
        const uint64_t max = (i % 5 == 0) ? 0xffffffffffffffffULL : uint64_t(i * 7 + 1);
        std::printf("seq[%02d] max=%20llu -> %20llu\n", i, (unsigned long long)max,
                    (unsigned long long)realm::fastrand(max, false));
    }

    std::printf("\n=== FastRand object ===\n");
    for (uint64_t seed : {1ULL, 2ULL, 12345ULL, 0xdeadbeefULL}) {
        realm::FastRand r(seed);
        std::printf("FastRand(%llu):", (unsigned long long)seed);
        for (int i = 0; i < 6; ++i)
            std::printf(" %llu", (unsigned long long)r(1000000));
        std::printf("\n");
    }

    std::printf("\n=== platform_timegm ===\n");
    // The interesting values straddle 2038-01-19 03:14:07 UTC, where the C++ cast to
    // int32_t wraps. A port that returned the full 64-bit time_t diverges only here.
    struct Case {
        int year, mon, mday, hour, min, sec;
    };
    const Case cases[] = {
        {70, 0, 1, 0, 0, 0},     // 1970-01-01 epoch
        {70, 0, 1, 0, 0, 1},     // epoch + 1
        {100, 0, 1, 0, 0, 0},    // 2000-01-01
        {119, 6, 15, 12, 30, 45},// 2019-07-15
        {138, 0, 19, 3, 14, 7},  // 2038-01-19 03:14:07 — INT32_MAX
        {138, 0, 19, 3, 14, 8},  // one second past: wraps negative
        {150, 0, 1, 0, 0, 0},    // 2050-01-01 — well past the wrap
        {200, 0, 1, 0, 0, 0},    // 2100-01-01
        {1, 0, 1, 0, 0, 0},      // 1901 — before the epoch
    };
    for (const Case& c : cases) {
        tm t;
        std::memset(&t, 0, sizeof(t));
        t.tm_year = c.year;
        t.tm_mon = c.mon;
        t.tm_mday = c.mday;
        t.tm_hour = c.hour;
        t.tm_min = c.min;
        t.tm_sec = c.sec;
        std::printf("timegm(y=%3d m=%d d=%2d %02d:%02d:%02d) = %20lld\n", c.year, c.mon, c.mday, c.hour, c.min, c.sec,
                    (long long)realm::platform_timegm(t));
    }

    return 0;
}
