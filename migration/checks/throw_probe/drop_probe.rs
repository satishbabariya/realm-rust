// Does a Rust `Drop` run when a C++ exception unwinds through the Rust frame?
//
// This decides whether `util/file_mapper::mmap` is portable. Its encryption branch is:
//
//     void* addr = mmap_anon(size);
//     ScopeExitFail cleanup([&]() noexcept { munmap(addr, size); });
//     mapping = file.encryption->add_mapping(page_start, addr, size, file.access);
//     return ...;
//
// i.e. "if add_mapping throws, munmap first". Reproducing that by *catching* is impossible
// -- Rust has no catch, and unlike throwing, no panic-mode setting changes that. But if
// the Itanium unwinder runs Rust's cleanup landing pads for a foreign exception, a Drop
// impl has exactly ScopeExitFail's semantics and no catch is needed.
//
// ScopeExitFail runs its lambda only on the exception path, so the guard here is armed by
// default and disarmed before a normal return.
#![crate_type = "staticlib"]
use core::ffi::{c_int, c_void};

extern "C-unwind" {
    /// Provided by the driver. Throws when `should_throw` is non-zero.
    fn cpp_may_throw(should_throw: c_int);
}

/// Observable side effect, so the driver can tell whether the guard ran.
#[no_mangle]
pub static mut CLEANUP_RAN: c_int = 0;

struct ScopeExitFail {
    armed: bool,
}

impl Drop for ScopeExitFail {
    fn drop(&mut self) {
        if self.armed {
            // Stands in for `munmap(addr, size)`.
            unsafe { CLEANUP_RAN += 1 };
        }
    }
}

/// Mirrors the shape of `mmap`'s encryption branch.
#[no_mangle]
pub unsafe extern "C-unwind" fn rust_scope_guard(should_throw: c_int) -> *mut c_void {
    let mut cleanup = ScopeExitFail { armed: true };
    cpp_may_throw(should_throw); // may unwind straight through this frame
    cleanup.armed = false; // reached only on success -- ScopeExitFail semantics
    core::ptr::null_mut()
}
