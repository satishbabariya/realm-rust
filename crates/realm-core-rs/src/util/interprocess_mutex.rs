//! Port of `upstream/src/realm/util/interprocess_mutex.cpp` (Apple path).
//!
//! Only `SemaphoreMutex` has out-of-line definitions in this translation unit; the
//! rest of `interprocess_mutex.hpp` is header-only. Seven exported symbols, all live:
//! the constructor and destructor in both their C1/C2 and D1/D2 forms, plus `lock`,
//! `unlock` and `try_lock`.
//!
//! ## Layout
//!
//! ```cpp
//! class SemaphoreMutex {
//!     dispatch_semaphore_t m_semaphore;   // one pointer, and the only member
//! };
//! ```
//!
//! The layout is fixed by the header, which every other translation unit still
//! compiles against, so this port does not get to choose it — `this` is simply a
//! pointer to a pointer. Nothing here reaches a `.realm`; the unit is
//! reachable-but-byte-invisible, so `make verify` can only show it broke nothing and
//! the differential carries the actual evidence.
//!
//! ## Why both C1/C2 and D1/D2
//!
//! The Itanium ABI emits a *complete-object* constructor (`C1`) and a *base-object*
//! constructor (`C2`), and likewise `D1`/`D2` for destructors. With no virtual bases
//! they are identical in behaviour, and the C++ compiler emits both symbols — so both
//! must exist here, or any caller that happens to reference the other form fails to
//! link. They delegate to one shared implementation rather than being duplicated.
//!
//! The unit is Apple-only upstream (`#if REALM_PLATFORM_APPLE`, with no `#else`), so
//! the port is too.

use core::ffi::c_void;

/// `dispatch_semaphore_t` — an opaque pointer.
type DispatchSemaphoreT = *mut c_void;

/// `DISPATCH_TIME_NOW`, confirmed by compiling against `<dispatch/dispatch.h>`.
const DISPATCH_TIME_NOW: u64 = 0;
/// `DISPATCH_TIME_FOREVER` — `~0ull`, confirmed the same way.
const DISPATCH_TIME_FOREVER: u64 = u64::MAX;

// libdispatch lives in libSystem, which is linked into every Mach-O binary.
#[cfg(target_vendor = "apple")]
extern "C" {
    fn dispatch_semaphore_create(value: isize) -> DispatchSemaphoreT;
    fn dispatch_semaphore_wait(sema: DispatchSemaphoreT, timeout: u64) -> isize;
    fn dispatch_semaphore_signal(sema: DispatchSemaphoreT) -> isize;
    fn dispatch_release(object: *mut c_void);
}

#[cfg(not(target_vendor = "apple"))]
compile_error!(
    "interprocess_mutex.rs ports only the REALM_PLATFORM_APPLE branch. Upstream has no \
     other implementation of SemaphoreMutex, so a non-Apple hybrid needs one written \
     before this unit can be used."
);

/// Shared body of the two constructor symbols.
///
/// # Safety
/// `this_ptr` must point to storage for one `dispatch_semaphore_t`.
#[inline]
unsafe fn construct(this_ptr: *mut DispatchSemaphoreT) {
    // `m_semaphore(dispatch_semaphore_create(1))` — a binary semaphore.
    *this_ptr = dispatch_semaphore_create(1);
}

/// Shared body of the two destructor symbols.
///
/// # Safety
/// `this_ptr` must point to a constructed `SemaphoreMutex`.
#[inline]
unsafe fn destruct(this_ptr: *mut DispatchSemaphoreT) {
    dispatch_release(*this_ptr);
}

/// `realm::util::SemaphoreMutex::SemaphoreMutex()` — complete-object constructor (C1).
///
/// # Safety
/// As [`construct`].
#[export_name = "_ZN5realm4util14SemaphoreMutexC1Ev"]
pub unsafe extern "C" fn semaphore_mutex_ctor_complete(this_ptr: *mut DispatchSemaphoreT) {
    construct(this_ptr);
}

/// `realm::util::SemaphoreMutex::SemaphoreMutex()` — base-object constructor (C2).
///
/// # Safety
/// As [`construct`].
#[export_name = "_ZN5realm4util14SemaphoreMutexC2Ev"]
pub unsafe extern "C" fn semaphore_mutex_ctor_base(this_ptr: *mut DispatchSemaphoreT) {
    construct(this_ptr);
}

/// `realm::util::SemaphoreMutex::~SemaphoreMutex()` — complete-object destructor (D1).
///
/// # Safety
/// As [`destruct`].
#[export_name = "_ZN5realm4util14SemaphoreMutexD1Ev"]
pub unsafe extern "C" fn semaphore_mutex_dtor_complete(this_ptr: *mut DispatchSemaphoreT) {
    destruct(this_ptr);
}

/// `realm::util::SemaphoreMutex::~SemaphoreMutex()` — base-object destructor (D2).
///
/// # Safety
/// As [`destruct`].
#[export_name = "_ZN5realm4util14SemaphoreMutexD2Ev"]
pub unsafe extern "C" fn semaphore_mutex_dtor_base(this_ptr: *mut DispatchSemaphoreT) {
    destruct(this_ptr);
}

/// `realm::util::SemaphoreMutex::lock()`
///
/// # Safety
/// `this_ptr` must point to a constructed `SemaphoreMutex`.
#[export_name = "_ZN5realm4util14SemaphoreMutex4lockEv"]
pub unsafe extern "C" fn semaphore_mutex_lock(this_ptr: *mut DispatchSemaphoreT) {
    dispatch_semaphore_wait(*this_ptr, DISPATCH_TIME_FOREVER);
}

/// `realm::util::SemaphoreMutex::try_lock()`
///
/// Returns true when the wait succeeded, i.e. when it returned 0. Note the C++ does
/// not treat "non-zero" as an error code — a timeout is the expected failure.
///
/// # Safety
/// `this_ptr` must point to a constructed `SemaphoreMutex`.
#[export_name = "_ZN5realm4util14SemaphoreMutex8try_lockEv"]
pub unsafe extern "C" fn semaphore_mutex_try_lock(this_ptr: *mut DispatchSemaphoreT) -> bool {
    dispatch_semaphore_wait(*this_ptr, DISPATCH_TIME_NOW) == 0
}

/// `realm::util::SemaphoreMutex::unlock()`
///
/// # Safety
/// `this_ptr` must point to a constructed `SemaphoreMutex`.
#[export_name = "_ZN5realm4util14SemaphoreMutex6unlockEv"]
pub unsafe extern "C" fn semaphore_mutex_unlock(this_ptr: *mut DispatchSemaphoreT) {
    dispatch_semaphore_signal(*this_ptr);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drives the exported symbols exactly as C++ would: construct into raw storage,
    /// exercise, then destroy.
    #[test]
    fn lock_unlock_and_try_lock_behave_like_a_binary_semaphore() {
        let mut storage: DispatchSemaphoreT = core::ptr::null_mut();
        let p = &mut storage as *mut DispatchSemaphoreT;

        // SAFETY: p points to storage for one dispatch_semaphore_t for the whole test.
        unsafe {
            semaphore_mutex_ctor_complete(p);
            assert!(!(*p).is_null(), "constructor must create the semaphore");

            // Free semaphore: try_lock succeeds.
            assert!(semaphore_mutex_try_lock(p));
            // Now held: a second try_lock must fail rather than block.
            assert!(!semaphore_mutex_try_lock(p));
            semaphore_mutex_unlock(p);
            // Released again.
            assert!(semaphore_mutex_try_lock(p));
            semaphore_mutex_unlock(p);

            // Blocking lock followed by unlock leaves the count balanced.
            semaphore_mutex_lock(p);
            semaphore_mutex_unlock(p);
            assert!(semaphore_mutex_try_lock(p));
            semaphore_mutex_unlock(p);

            semaphore_mutex_dtor_complete(p);
        }
    }

    /// The C1/C2 and D1/D2 pairs must be interchangeable — they are the same function
    /// in the Itanium ABI when there are no virtual bases.
    #[test]
    fn base_and_complete_object_variants_agree() {
        let mut storage: DispatchSemaphoreT = core::ptr::null_mut();
        let p = &mut storage as *mut DispatchSemaphoreT;

        // SAFETY: construct with C2 and destroy with D2, the mirror of the test above.
        unsafe {
            semaphore_mutex_ctor_base(p);
            assert!(!(*p).is_null());
            assert!(semaphore_mutex_try_lock(p));
            semaphore_mutex_unlock(p);
            semaphore_mutex_dtor_base(p);
        }
    }
}
