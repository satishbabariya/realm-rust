//! Throwing C++ exceptions from Rust.
//!
//! Not a ported unit — `upstream/src/realm/exceptions.cpp` **stays C++** and this depends
//! on that. Because it does, every part of a `realm::LogicError` already exists in the
//! link and none of it has to be synthesized:
//!
//! ```text
//! T __ZN5realm10LogicErrorC1ENS_10ErrorCodes5ErrorENSt3__117basic_string_viewIcNS3_...EE
//! T __ZN5realm10LogicErrorD1Ev
//! S __ZTIN5realm10LogicErrorE
//! ```
//!
//! So "throwing from Rust" is an `__cxa_allocate_exception`, a call to the bound
//! constructor, and `__cxa_throw` with the bound typeinfo. That is the whole shim. Six
//! park files previously assumed this required synthesizing 102 RTTI records; it does not,
//! and `migration/checks/throw_probe/` demonstrates the working version end to end —
//! C++ catches it by reference as `realm::LogicError` with the right code and message.
//!
//! What it *does* require is `panic = "unwind"`, since the unwind passes back through the
//! Rust frame. See the rationale in `Cargo.toml` and the abort-on-panic hook in `lib.rs`
//! that preserves loud failure for genuine Rust bugs.

use core::ffi::{c_char, c_int, c_void};

use crate::array_unsigned::Allocator;

/// `std::string_view` — `{const char*, size_t}`.
#[repr(C)]
struct StringView {
    data: *const c_char,
    size: usize,
}

extern "C" {
    fn __cxa_allocate_exception(size: usize) -> *mut c_void;

    /// `realm::LogicError::LogicError(ErrorCodes::Error, std::string_view)`
    #[link_name = "_ZN5realm10LogicErrorC1ENS_10ErrorCodes5ErrorENSt3__117basic_string_viewIcNS3_11char_traitsIcEEEE"]
    fn logic_error_ctor(this: *mut c_void, code: c_int, msg: StringView);

    /// `realm::LogicError::~LogicError()` — handed to `__cxa_throw` as the destructor.
    #[link_name = "_ZN5realm10LogicErrorD1Ev"]
    fn logic_error_dtor(this: *mut c_void);

    /// `typeinfo for realm::LogicError`, emitted by `exceptions.cpp`.
    #[link_name = "_ZTIN5realm10LogicErrorE"]
    static LOGIC_ERROR_TYPEINFO: c_void;
}

extern "C-unwind" {
    fn __cxa_throw(exc: *mut c_void, tinfo: *const c_void, dest: unsafe extern "C" fn(*mut c_void)) -> !;
}

/// `sizeof(realm::LogicError)`, measured. `Exception` is the same size; both are a vptr
/// plus a `Status`-carrying pointer.
const SIZEOF_LOGIC_ERROR: usize = 16;

/// `ErrorCodes::WrongTransactionState`
const WRONG_TRANSACTION_STATE: c_int = 1015;

/// The message `alloc.hpp:495` and `alloc.hpp:507` both use, byte for byte.
const READ_ONLY_MSG: &str = "Trying to modify database while in read transaction";

/// Throw `realm::LogicError(ErrorCodes::WrongTransactionState, ...)`, exactly as the
/// inline `Allocator::alloc` / `Allocator::realloc_` do when the allocator is read-only.
///
/// `#[inline(never)]` so the throw site stays one identifiable frame, which is what makes
/// a backtrace from a hybrid build readable.
#[inline(never)]
pub(crate) unsafe fn throw_wrong_transaction_state() -> ! {
    let e = __cxa_allocate_exception(SIZEOF_LOGIC_ERROR);
    logic_error_ctor(
        e,
        WRONG_TRANSACTION_STATE,
        StringView {
            data: READ_ONLY_MSG.as_ptr() as *const c_char,
            size: READ_ONLY_MSG.len(),
        },
    );
    __cxa_throw(e, &LOGIC_ERROR_TYPEINFO as *const c_void, logic_error_dtor)
}

/// `Allocator::m_is_read_only` — the whole-allocator flag at offset 56, set by
/// `set_read_only()`. Distinct from `Allocator::is_read_only(ref)`, which asks whether one
/// *ref* is below the baseline; `alloc.hpp` uses the flag, not the ref test, to decide
/// whether to throw.
#[inline]
pub(crate) unsafe fn allocator_is_read_only_flag(alloc: *const Allocator) -> bool {
    *(alloc as *const u8).add(56) != 0
}

/// `Allocator::is_read_only(ref)` — re-exported from `array_unsigned`, which measured it
/// first, so there is one definition rather than two.
#[inline]
pub(crate) unsafe fn allocator_is_read_only_ref(alloc: *const Allocator, ref_: usize) -> bool {
    crate::array_unsigned::allocator_is_read_only(alloc, ref_)
}
