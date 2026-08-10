// Drives drop_probe.rs: does a Rust Drop run when a C++ exception unwinds through?
#include <realm/exceptions.hpp>

#include <cstdio>

extern "C" void* rust_scope_guard(int should_throw);
extern "C" int CLEANUP_RAN;

extern "C" void cpp_may_throw(int should_throw)
{
    if (should_throw)
        throw realm::AddressSpaceExhausted("add_mapping() failed");
}

int main()
{
    int failures = 0;

    // 1. Success path: ScopeExitFail must NOT run its lambda.
    CLEANUP_RAN = 0;
    rust_scope_guard(0);
    if (CLEANUP_RAN != 0) {
        std::printf("  success path      FAIL  cleanup ran %d times, expected 0\n", CLEANUP_RAN);
        ++failures;
    }
    else {
        std::printf("  success path      ok    cleanup did not run\n");
    }

    // 2. Exception path: the Rust Drop must run exactly once, and the exception must
    //    still arrive here intact.
    CLEANUP_RAN = 0;
    bool caught = false;
    try {
        rust_scope_guard(1);
    }
    catch (const realm::AddressSpaceExhausted&) {
        caught = true;
    }
    catch (...) {
        std::printf("  exception path    FAIL  wrong exception type propagated\n");
        ++failures;
    }

    if (!caught) {
        std::printf("  exception path    FAIL  exception did not propagate through Rust\n");
        ++failures;
    }
    else if (CLEANUP_RAN != 1) {
        std::printf("  exception path    FAIL  cleanup ran %d times, expected 1 -- Rust drops do\n"
                    "                          NOT run for a foreign unwind\n",
                    CLEANUP_RAN);
        ++failures;
    }
    else {
        std::printf("  exception path    ok    cleanup ran once and the exception propagated\n");
    }

    std::printf("done\n");
    return failures == 0 ? 0 : 1;
}
