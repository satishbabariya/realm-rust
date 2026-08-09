// Differential driver for util/interprocess_mutex.cpp (SemaphoreMutex).
//
// Compiled twice by run_interprocess_mutex_differential.sh — once against the oracle's
// interprocess_mutex.cpp.o, once against the Rust staticlib — and the two dumps must be
// byte-identical.
//
// Nothing here reaches a .realm, so diff-test cannot see this unit at all. What the
// driver pins down is the observable contract of a binary semaphore: that a fresh mutex
// is unlocked, that try_lock reports contention rather than blocking, that lock/unlock
// keep the count balanced, and that the C1/C2 and D1/D2 ABI variants are
// interchangeable.

#include <cstdio>
#include <realm/util/interprocess_mutex.hpp>

using realm::util::SemaphoreMutex;

int main()
{
    std::printf("=== fresh mutex ===\n");
    {
        SemaphoreMutex m;
        std::printf("try_lock on fresh      = %d\n", int(m.try_lock()));
        std::printf("try_lock while held    = %d\n", int(m.try_lock()));
        m.unlock();
        std::printf("try_lock after unlock  = %d\n", int(m.try_lock()));
        m.unlock();
    }

    std::printf("\n=== blocking lock / unlock balance ===\n");
    {
        SemaphoreMutex m;
        for (int i = 0; i < 5; ++i) {
            m.lock();
            m.unlock();
            std::printf("round %d: try_lock=%d ", i, int(m.try_lock()));
            m.unlock();
            std::printf("released\n");
        }
    }

    std::printf("\n=== many independent mutexes are independent ===\n");
    {
        SemaphoreMutex a;
        SemaphoreMutex b;
        std::printf("a.try_lock=%d b.try_lock=%d\n", int(a.try_lock()), int(b.try_lock()));
        std::printf("a held, b held: a.try_lock=%d b.try_lock=%d\n", int(a.try_lock()), int(b.try_lock()));
        a.unlock();
        std::printf("a released:     a.try_lock=%d b.try_lock=%d\n", int(a.try_lock()), int(b.try_lock()));
        a.unlock();
        b.unlock();
    }

    std::printf("\n=== construct/destroy repeatedly ===\n");
    for (int i = 0; i < 8; ++i) {
        SemaphoreMutex m;
        const int t = int(m.try_lock());
        m.unlock();
        std::printf("iteration %d: try_lock=%d\n", i, t);
    }

    std::printf("\ndone\n");
    return 0;
}
