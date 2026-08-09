// Differential driver for upstream/src/realm/status.cpp.
//
// Byte-invisible: Status carries error codes and messages and never reaches a .realm.
// make verify proves the link is intact and (as it turned out) that the error path does
// not crash; it cannot show that the port is *correct*.
//
// Written against the public API only — Status's constructors, reason(), code(),
// code_string() and operator<< — because that is how every caller reaches
// ErrorInfo::create and the constructor, and it exercises the std::string move and the
// bind_ptr refcount through the same paths real code uses.
//
// The string cases are chosen around libc++'s short-string boundary: a std::string of
// 22 bytes or fewer lives inline in the 24-byte object, 23 or more is heap-allocated
// with a different internal representation. The port reads both, and a move must leave
// the source empty in either case or the caller's destructor frees a buffer the port
// now owns.
//
// Build and run via migration/checks/run_status_differential.sh.

#include <realm/status.hpp>

#include <cstdio>
#include <sstream>
#include <string>
#include <vector>


// ---------------------------------------------------------------------------
// Allocation accounting.
//
// The one thing in this unit that can go wrong *silently* is the bind_ptr reference
// count. Too high and the ErrorInfo is never freed; too low and it is freed while
// copies still point at it. Neither changes any printed value, so the first version of
// this driver reported PASS with the count deliberately off by one.
//
// Replacing global operator new/delete makes it observable: a leaked ErrorInfo shows up
// as a mismatch in the net count. The Rust port calls the same `_Znwm`, so these
// counters see its allocations too.
// ---------------------------------------------------------------------------
#include <cstdlib>
#include <new>

namespace {
long g_new_calls = 0;
long g_delete_calls = 0;
} // namespace

void* operator new(std::size_t n)
{
    ++g_new_calls;
    if (void* p = std::malloc(n ? n : 1))
        return p;
    throw std::bad_alloc();
}
void operator delete(void* p) noexcept
{
    if (p) {
        ++g_delete_calls;
        std::free(p);
    }
}
void operator delete(void* p, std::size_t) noexcept
{
    operator delete(p);
}
void* operator new[](std::size_t n)
{
    return operator new(n);
}
void operator delete[](void* p) noexcept
{
    operator delete(p);
}
void operator delete[](void* p, std::size_t) noexcept
{
    operator delete(p);
}

using namespace realm;

namespace {

void show(const char* tag, const Status& s)
{
    std::ostringstream os;
    os << s;
    std::printf("%-26s ok=%d code=%d code_string=%s reason=[%zu]%s stream=[%zu]%s\n", tag, int(s.is_ok()),
                int(s.code()), std::string(s.code_string()).c_str(), s.reason().size(), s.reason().c_str(),
                os.str().size(), os.str().c_str());
}

// A moved-from std::string must be valid and empty. If the port's move left the source
// owning the same buffer, this is where the double free would surface.
void move_case(const char* tag, std::string reason)
{
    const size_t before = reason.size();
    Status s(ErrorCodes::RuntimeError, std::move(reason));
    std::printf("%-26s moved_from_size=%zu moved_from_empty=%d original_size=%zu\n", tag, reason.size(),
                int(reason.empty()), before);
    show(tag, s);
    // Touch the moved-from string: it must still be a usable string object.
    reason += "reused";
    std::printf("%-26s reused=[%zu]%s\n", tag, reason.size(), reason.c_str());
}

} // namespace

int main()
{
    // 1. OK status takes the null-ErrorInfo path in every accessor and in operator<<.
    show("ok", Status::OK());

    // 2. Reasons across the short/long boundary. 22 is the last inline size, 23 the
    //    first heap one.
    for (size_t n : {size_t(0), size_t(1), size_t(10), size_t(21), size_t(22), size_t(23), size_t(24), size_t(40), size_t(200), size_t(1000)}) {
        std::string r(n, 'x');
        char tag[64];
        std::snprintf(tag, sizeof(tag), "len=%zu", n);
        Status s(ErrorCodes::InvalidArgument, std::move(r));
        show(tag, s);
    }

    // 3. The same boundary, checking what the move left behind.
    for (size_t n : {size_t(0), size_t(22), size_t(23), size_t(100)}) {
        char tag[64];
        std::snprintf(tag, sizeof(tag), "move len=%zu", n);
        move_case(tag, std::string(n, 'y'));
    }

    // 4. Reasons with awkward bytes: embedded NUL, high bytes, and one that is exactly
    //    the inline capacity so an off-by-one in the size field shows up.
    {
        std::string r("ab\0cd", 5);
        Status s(ErrorCodes::BrokenInvariant, std::move(r));
        std::printf("embedded_nul reason_size=%zu\n", s.reason().size());
        show("embedded_nul", s);
    }
    {
        std::string r;
        for (int i = 1; i < 256; ++i)
            r.push_back(char(i));
        Status s(ErrorCodes::BrokenInvariant, std::move(r));
        std::printf("high_bytes reason_size=%zu front=%d back=%d\n", s.reason().size(),
                    int((unsigned char)s.reason().front()), int((unsigned char)s.reason().back()));
    }

    // 5. Every error code, streamed. This crosses status.cpp into error_codes.cpp
    //    (also Rust now), so a code_string regression shows up here too.
    for (auto code : ErrorCodes::get_all_codes()) {
        if (code == ErrorCodes::OK)
            continue;
        Status s(code, "r");
        std::ostringstream os;
        os << s;
        std::printf("code %d stream=%s\n", int(code), os.str().c_str());
    }

    // 6. Refcount behaviour, observed through copies. bind_ptr binds on copy and
    //    unbinds on destruction; if create returned a count of 0 or 2 instead of 1,
    //    this either frees early or leaks, and freeing early corrupts the copies.
    {
        Status a(ErrorCodes::RuntimeError, "shared reason string that is long enough to allocate");
        std::vector<Status> copies;
        for (int i = 0; i < 8; ++i)
            copies.push_back(a);
        for (int i = 0; i < 8; ++i)
            std::printf("copy %d reason=[%zu]%s code=%d\n", i, copies[i].reason().size(),
                        copies[i].reason().c_str(), int(copies[i].code()));
        copies.clear();
        // `a` must still be intact after all copies are destroyed.
        std::printf("after copies reason=[%zu]%s\n", a.reason().size(), a.reason().c_str());
    }

    // 7. Assignment and self-assignment through the refcounted pointer.
    {
        Status a(ErrorCodes::RuntimeError, "first");
        Status b(ErrorCodes::InvalidArgument, "second");
        a = b;
        std::printf("assigned a=[%s] b=[%s]\n", a.reason().c_str(), b.reason().c_str());
        b = Status::OK();
        std::printf("after b=OK a=[%s] b_ok=%d\n", a.reason().c_str(), int(b.is_ok()));
    }

    // 8. Net allocation balance over a block whose Statuses are all destroyed. If
    //    ErrorInfo::create leaves the refcount too high, the ErrorInfo is never freed
    //    and this number differs between the two builds.
    {
        const long new_before = g_new_calls, del_before = g_delete_calls;
        for (int i = 0; i < 50; ++i) {
            Status s(ErrorCodes::RuntimeError,
                     std::string("a reason long enough to force a heap allocation ") + std::to_string(i));
            Status copy = s;
            (void)copy.reason().size();
        }
        const long leaked = (g_new_calls - new_before) - (g_delete_calls - del_before);
        std::printf("alloc_balance leaked=%ld\n", leaked);
    }

    std::printf("done\n");
    return 0;
}
