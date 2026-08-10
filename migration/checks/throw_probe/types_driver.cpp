// Catch each Rust-thrown exception AS ITS OWN TYPE.
//
// The derived-type catch is the whole point. `AddressSpaceExhausted` is built by calling
// its `RuntimeError` base constructor and then overwriting the vptr; catching it as
// `RuntimeError&` would succeed even if that fixup were wrong or missing, so the ordering
// below puts the derived catch first and treats a base-class catch as a FAILURE.
#include <realm/exceptions.hpp>

#include <cstdio>
#include <cstring>
#include <stdexcept>
#include <system_error>
#include <typeinfo>

extern "C" void throw_address_space_exhausted(const char*, size_t);
extern "C" void throw_realm_system_error(int, const char*, size_t);
extern "C" void throw_std_system_error(int, const char*, size_t);
extern "C" void throw_std_runtime_error(const char*, size_t);

static int failures = 0;

static void check(const char* label, bool ok, const char* detail)
{
    std::printf("  %-28s %s  %s\n", label, ok ? "ok  " : "FAIL", detail);
    if (!ok)
        ++failures;
}

int main()
{
    const char* msg = "mmap() failed: Cannot allocate memory (size: 4096, offset: 0)";
    const size_t len = std::strlen(msg);

    // 1. realm::AddressSpaceExhausted -- the vptr-fixup case.
    try {
        throw_address_space_exhausted(msg, len);
        check("AddressSpaceExhausted", false, "no exception propagated");
    }
    catch (const realm::AddressSpaceExhausted& e) {
        // `typeid` on a reference to a polymorphic type reads the object's VPTR, unlike
        // the catch itself, which matches on the typeinfo handed to __cxa_throw. Without
        // this line a missing vptr fixup is undetectable here -- a control that removed
        // the fixup passed all four cases before it was added.
        bool right_dynamic_type = typeid(e) == typeid(realm::AddressSpaceExhausted);
        bool ok = std::strcmp(e.what(), msg) == 0 && e.code() == realm::ErrorCodes::AddressSpaceExhausted &&
                  right_dynamic_type;
        check("AddressSpaceExhausted", ok, right_dynamic_type ? e.what() : "vptr says a different dynamic type");
    }
    catch (const realm::RuntimeError&) {
        // Reached only if the vptr/typeinfo fixup did not take: the object is still a
        // RuntimeError as far as RTTI is concerned.
        check("AddressSpaceExhausted", false, "caught as RuntimeError -- vptr fixup did not take");
    }
    catch (...) {
        check("AddressSpaceExhausted", false, "caught as something else entirely");
    }

    // 2. realm::SystemError
    try {
        throw_realm_system_error(ENOMEM, msg, len);
        check("realm::SystemError", false, "no exception propagated");
    }
    catch (const realm::SystemError& e) {
        check("realm::SystemError", std::strstr(e.what(), "mmap()") != nullptr, e.what());
    }
    catch (...) {
        check("realm::SystemError", false, "wrong type");
    }

    // 3. std::system_error
    try {
        throw_std_system_error(ENOMEM, msg, len);
        check("std::system_error", false, "no exception propagated");
    }
    catch (const std::system_error& e) {
        bool ok = e.code().value() == ENOMEM && std::strstr(e.what(), "mmap()") != nullptr;
        check("std::system_error", ok, e.what());
    }
    catch (...) {
        check("std::system_error", false, "wrong type");
    }

    // 4. std::runtime_error -- checked AFTER system_error, because system_error derives
    // from it and a catch order that put this first would mask a wrong typeinfo above.
    try {
        throw_std_runtime_error(msg, len);
        check("std::runtime_error", false, "no exception propagated");
    }
    catch (const std::system_error&) {
        check("std::runtime_error", false, "caught as std::system_error -- wrong typeinfo");
    }
    catch (const std::runtime_error& e) {
        check("std::runtime_error", std::strcmp(e.what(), msg) == 0, e.what());
    }
    catch (...) {
        check("std::runtime_error", false, "wrong type");
    }

    std::printf("done\n");
    return failures == 0 ? 0 : 1;
}
