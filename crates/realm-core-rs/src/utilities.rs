//! Port of `upstream/src/realm/utilities.cpp`.
//!
//! Nine exported symbols, all live: two mutable globals (`sse_support`,
//! `avx_support`), `cpuid_init`, `fast_popcount32/64`, `fastrand`,
//! `FastRand::operator()`, `millisleep`, and `platform_timegm`.
//!
//! This is the first ported unit that exports **data** symbols and a **member
//! function**, not just free functions.
//!
//! ## Fidelity notes
//!
//! - **`platform_timegm` truncates to 32 bits.** The C++ is
//!   `int64_t(static_cast<int32_t>(timegm(&time)))`, so timestamps past 2038-01-19
//!   wrap negative. Reproduced exactly; returning the full 64-bit `time_t` would be
//!   more correct and would disagree with every file the C++ has ever written.
//! - **`fastrand`'s modulus special-cases `max == u64::MAX`.** `max + 1` overflows to
//!   0, and the C++ substitutes `0xffff_ffff_ffff_ffff`. In C++ the unsigned overflow
//!   is defined; in Rust the same expression would panic in debug, so the check is
//!   written against `checked_add`.
//! - **`cpuid_init` sets `avx_support = -1` unconditionally under clang.** The AVX
//!   detection block is guarded by `#if !defined __clang__ && ...`, so on this
//!   toolchain `avxSupported` is always false. Mirrored rather than "fixed" — a hybrid
//!   that reported AVX where the oracle does not would select different code paths in
//!   every TU that inlines `cpu_avx<>()`.
//! - **`fast_popcount32` is a byte-table sum in the C++**, not an intrinsic (the
//!   intrinsic path is deliberately disabled upstream — see the comment above it). It
//!   sums `a_popcount_bits` over the four bytes of the value, which is exactly the
//!   population count of the 32-bit word. `count_ones()` is used here instead of
//!   transcribing a 256-entry table, because the two are equal by construction and
//!   copying the table by hand is the more likely source of an error. The differential
//!   sweeps signed, unsigned, and boundary values to confirm it.
//!
//! `realm::util::Mutex::~Mutex()` is emitted by the C++ TU as a `weak private
//! external` and is defined in no other object in the Storage archive; it exists only
//! to destroy this unit's file-static `fastrand_mutex`. Nothing outside the TU
//! references it, so the Rust port does not provide it.

use core::ffi::{c_char, c_int, c_long};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// `realm::sse_support` — `signed char`, initialised to -1 before `cpuid_init()` runs.
///
/// Read by `cpu_sse<>()` in `utilities.hpp`, which is inlined into many other
/// translation units, so this symbol must exist with byte-identical semantics.
#[export_name = "_ZN5realm11sse_supportE"]
pub static mut SSE_SUPPORT: i8 = -1;

/// `realm::avx_support` — `signed char`, initialised to -1.
#[export_name = "_ZN5realm11avx_supportE"]
pub static mut AVX_SUPPORT: i8 = -1;

/// `realm::cpuid_init()`
///
/// Called explicitly from `group.cpp:47`, not from a static initialiser, so replacing
/// this translation unit does not change when it runs.
///
/// # Safety
/// Writes the two global support bytes. The C++ notes that the race here is benign
/// because a byte write is atomic on the target.
#[export_name = "_ZN5realm10cpuid_initEv"]
pub unsafe extern "C" fn cpuid_init() {
    #[cfg(target_arch = "x86_64")]
    {
        // The C++ runs `cpuid` with eax=1 and keeps ecx.
        let cret = core::arch::x86_64::__cpuid(1).ecx;

        let sse = if cret & 0x0010_0000 != 0 {
            1i8 // SSE 4.2
        } else if cret & 0x1 != 0 {
            0i8 // SSE 3
        } else {
            -2i8
        };
        core::ptr::write(core::ptr::addr_of_mut!(SSE_SUPPORT), sse);

        // The AVX probe is compiled out under clang (`#if !defined __clang__ && ...`),
        // so `avxSupported` is always false and this is always -1.
        core::ptr::write(core::ptr::addr_of_mut!(AVX_SUPPORT), -1i8);
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        // REALM_COMPILER_SSE is x86-only; on other architectures the C++ body is empty
        // and both globals keep their static initialisers.
    }
}

/// `realm::fast_popcount32(int32_t)`
#[export_name = "_ZN5realm15fast_popcount32Ei"]
pub extern "C" fn fast_popcount32(x: i32) -> c_int {
    // Equal to the C++ byte-table sum; see the module note on why the table is not
    // transcribed.
    (x as u32).count_ones() as c_int
}

/// `realm::fast_popcount64(int64_t)`
#[export_name = "_ZN5realm15fast_popcount64Ex"]
pub extern "C" fn fast_popcount64(x: i64) -> c_int {
    // The C++ splits into two 32-bit halves and sums; identical to a 64-bit popcount.
    (x as u64).count_ones() as c_int
}

/// Shared xorshift state for [`fastrand`]. The C++ uses a function-local
/// `static std::atomic<uint64_t> state(1)` guarded by a file-static mutex.
static FASTRAND_STATE: AtomicU64 = AtomicU64::new(1);
/// Mirrors `fastrand_mutex`. The C++ comment explains it exists to keep Helgrind and
/// tsan quiet rather than for correctness; it is kept so the locking behaviour matches.
static FASTRAND_MUTEX: Mutex<()> = Mutex::new(());

/// The xorshift step and scaling shared by `fastrand` and `FastRand::operator()`.
#[inline]
fn xorshift_scale(mut x: u64, max: u64) -> (u64, u64) {
    x ^= x >> 12; // a
    x ^= x << 25; // b
    x ^= x >> 27; // c
    // `(x * 2685821657736338717) % (max + 1 == 0 ? UINT64_MAX : max + 1)`.
    // The multiply wraps in C++ (unsigned) and must wrap here too.
    let scaled = x.wrapping_mul(2685821657736338717);
    let modulus = match max.checked_add(1) {
        Some(m) => m,
        None => 0xffff_ffff_ffff_ffff, // max == UINT64_MAX; C++ relies on the overflow
    };
    (x, scaled % modulus)
}

/// `realm::fastrand(uint64_t max, bool is_seed)`
#[export_name = "_ZN5realm8fastrandEyb"]
pub extern "C" fn fastrand(max: u64, is_seed: bool) -> u64 {
    // Poisoning cannot happen: nothing in the guarded region can panic.
    let _lg = FASTRAND_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

    // Thread-safe increment so two callers at the same instant differ.
    FASTRAND_STATE.fetch_add(1, Ordering::Release);
    let seed = if is_seed {
        max
    } else {
        FASTRAND_STATE.load(Ordering::Acquire)
    };
    let (state, result) = xorshift_scale(seed, max);
    FASTRAND_STATE.store(state, Ordering::Release);
    result
}

/// `realm::FastRand::operator()(uint64_t max)`
///
/// `class FastRand { uint64_t m_state; }` — one member, so `this` points directly at
/// the state word.
///
/// # Safety
/// `this_ptr` must point to a live `FastRand`, as guaranteed by the C++ caller.
#[export_name = "_ZN5realm8FastRandclEy"]
pub unsafe extern "C" fn fast_rand_call(this_ptr: *mut u64, max: u64) -> u64 {
    let (state, result) = xorshift_scale(*this_ptr, max);
    *this_ptr = state;
    result
}

/// `realm::millisleep(unsigned long)`
#[export_name = "_ZN5realm10millisleepEm"]
pub extern "C" fn millisleep(milliseconds: u64) {
    // The C++ uses nanosleep(): secs = ms / 1000, nsec = (ms % 1000) * 1_000_000.
    let secs = milliseconds / 1000;
    let nanos = (milliseconds % 1000) * 1_000_000;
    std::thread::sleep(std::time::Duration::new(secs, nanos as u32));
}

/// `struct tm` as laid out by the Darwin C library. Passed **by value** to
/// `platform_timegm`, so the field order and the two BSD extension members at the end
/// are part of the ABI.
#[repr(C)]
pub struct Tm {
    tm_sec: c_int,
    tm_min: c_int,
    tm_hour: c_int,
    tm_mday: c_int,
    tm_mon: c_int,
    tm_year: c_int,
    tm_wday: c_int,
    tm_yday: c_int,
    tm_isdst: c_int,
    tm_gmtoff: c_long,
    tm_zone: *mut c_char,
}

extern "C" {
    fn timegm(tm: *mut Tm) -> i64;
}

/// `realm::platform_timegm(tm)`
///
/// # Safety
/// `time` is a by-value `struct tm` from the C++ caller; `timegm` may normalise the
/// local copy, which the caller never observes because it passed by value.
#[export_name = "_ZN5realm15platform_timegmE2tm"]
pub unsafe extern "C" fn platform_timegm(mut time: Tm) -> i64 {
    let unix_time = timegm(&mut time as *mut Tm);
    // `int64_t(static_cast<int32_t>(unix_time))` — deliberate 32-bit truncation.
    // Timestamps past 2038 wrap negative, matching every file the C++ has written.
    (unix_time as i32) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popcount_matches_for_signed_and_boundary_values() {
        for v in [0i32, 1, -1, i32::MIN, i32::MAX, 0x5555_5555u32 as i32, -2] {
            assert_eq!(fast_popcount32(v), (v as u32).count_ones() as c_int);
        }
        for v in [0i64, 1, -1, i64::MIN, i64::MAX, -2] {
            assert_eq!(fast_popcount64(v), (v as u64).count_ones() as c_int);
        }
        // The C++ splits a 64-bit value into halves; check the halves really sum.
        let v: i64 = 0x0123_4567_89ab_cdef;
        assert_eq!(
            fast_popcount64(v),
            fast_popcount32(v as i32) + fast_popcount32((v >> 32) as i32)
        );
    }

    #[test]
    fn fastrand_respects_the_bound() {
        for max in [0u64, 1, 2, 255, 1000] {
            for _ in 0..64 {
                assert!(fastrand(max, false) <= max, "exceeded bound {max}");
            }
        }
    }

    /// `max == u64::MAX` makes `max + 1` overflow; the C++ substitutes UINT64_MAX as
    /// the modulus. This is the case that would panic in debug if written naively.
    #[test]
    fn fastrand_handles_the_overflowing_modulus() {
        let v = fastrand(u64::MAX, false);
        assert!(v < 0xffff_ffff_ffff_ffff);
    }

    #[test]
    fn seeded_fastrand_is_a_pure_function_of_the_seed() {
        // is_seed = true ignores the shared state, so two calls with the same seed
        // must agree even though the shared counter advanced in between.
        let a = fastrand(12345, true);
        let _ = fastrand(999, false);
        let b = fastrand(12345, true);
        assert_eq!(a, b);
    }

    #[test]
    fn fast_rand_object_advances_its_own_state() {
        let mut state: u64 = 1;
        // SAFETY: state is a live u64, which is exactly FastRand's layout.
        let first = unsafe { fast_rand_call(&mut state as *mut u64, u64::MAX) };
        let after_first = state;
        // SAFETY: as above.
        let second = unsafe { fast_rand_call(&mut state as *mut u64, u64::MAX) };
        assert_ne!(after_first, 1, "state must advance");
        assert_ne!(first, second, "successive draws must differ");
    }
}
