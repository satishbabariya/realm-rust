//! Port of `upstream/src/realm/status.cpp`.
//!
//! Reachable but **byte-invisible**: `Status` carries error codes and messages and
//! never reaches a `.realm`. `make verify` proves the link is intact; the evidence is
//! `migration/checks/run_status_differential.sh`.
//!
//! 47 lines and three functions, but two of them touch `std::string`'s internal
//! representation, which is the delicate part and the reason this file is mostly
//! comments about libc++ rather than about realm.
//!
//! # Layouts, measured with `-fdump-record-layouts`
//!
//! ```text
//! struct realm::Status::ErrorInfo                       [sizeof=32, align=8]
//!   0 | atomic<uint32_t>        m_refs
//!   4 | const ErrorCodes::Error m_code
//!   8 | std::string             m_reason
//!
//! class realm::util::bind_ptr<ErrorInfo>                [sizeof=8]
//!   0 | ErrorInfo* m_ptr        (bind_ptr_base is empty)
//!
//! class realm::Status                                   [sizeof=8]
//!   0 | bind_ptr<ErrorInfo> m_error
//! ```
//!
//! # `std::string` in this libc++
//!
//! 24 bytes, a union of a short and a long representation, and **`__is_long_` is the
//! LOW bit** — older libc++ put it in the high bit of the first word, and code written
//! from memory of that layout reads every short string as long:
//!
//! ```text
//! short:  byte 0 : bit 0 = __is_long_ (0), bits 1..7 = __size_
//!         bytes 1..23 = __data_          (22 chars + NUL)
//! long:   word 0 : bit 0 = __is_long_ (1), bits 1..63 = __cap_
//!         word 1 = __size_
//!         word 2 = __data_ (pointer)
//! ```
//!
//! This port only ever **reads** a string and **moves** one. It never allocates,
//! reallocates or frees string storage, which keeps it clear of the parts of the
//! representation that are easy to get wrong.

use core::ffi::{c_char, c_void};
use core::sync::atomic::{AtomicU32, Ordering};

/// `std::string`, treated as 24 opaque bytes plus accessors. Deliberately not modelled
/// field by field: the union is version-dependent and nothing here needs to construct
/// one.
#[repr(C)]
pub struct StdString {
    rep: [u8; 24],
}

impl StdString {
    /// `(data, size)`. Mirrors `basic_string::data()` / `size()`.
    #[inline]
    unsafe fn parts(&self) -> (*const c_char, usize) {
        let base = self.rep.as_ptr();
        if (*base) & 1 == 0 {
            // short: size in bits 1..7 of byte 0, data starts at byte 1
            (base.add(1) as *const c_char, ((*base) >> 1) as usize)
        } else {
            let words = base as *const usize;
            let size = words.add(1).read_unaligned();
            let data = words.add(2).read_unaligned() as *const c_char;
            (data, size)
        }
    }
}

/// `realm::Status::ErrorInfo`.
#[repr(C)]
pub struct ErrorInfo {
    m_refs: AtomicU32,
    m_code: i32,
    m_reason: StdString,
}

/// `realm::util::bind_ptr<ErrorInfo>` — one pointer, returned in `rax`.
#[repr(C)]
pub struct BindPtr {
    m_ptr: *mut ErrorInfo,
}

/// `realm::Status` — one `bind_ptr`.
#[repr(C)]
pub struct Status {
    m_error: BindPtr,
}

const _: () = {
    assert!(core::mem::size_of::<StdString>() == 24);
    assert!(core::mem::size_of::<ErrorInfo>() == 32);
    assert!(core::mem::offset_of!(ErrorInfo, m_code) == 4);
    assert!(core::mem::offset_of!(ErrorInfo, m_reason) == 8);
    assert!(core::mem::size_of::<BindPtr>() == 8);
    assert!(core::mem::size_of::<Status>() == 8);
};

extern "C-unwind" {
    /// `operator new(unsigned long)`.
    #[link_name = "_Znwm"]
    fn cxx_operator_new(size: usize) -> *mut u8;

    /// `std::__put_character_sequence<char, char_traits<char>>(ostream&, const char*, size_t)`
    #[link_name = "_ZNSt3__124__put_character_sequenceB9nqe210106IcNS_11char_traitsIcEEEERNS_13basic_ostreamIT_T0_EES7_PKS4_m"]
    fn put_character_sequence(os: *mut c_void, s: *const c_char, len: usize) -> *mut c_void;
}

extern "C" {
    /// `realm::ErrorCodes::error_string(Error)` — supplied by `crate::error_codes`,
    /// which is itself Rust. Declared rather than called directly so the C++ mangled
    /// entry point stays the single definition of that behaviour.
    #[link_name = "_ZN5realm10ErrorCodes12error_stringENS0_5ErrorE"]
    fn error_codes_error_string(code: i32) -> StringView;
}

/// `std::string_view` as returned by `error_string`.
#[repr(C)]
struct StringView {
    data: *const c_char,
    size: usize,
}

/// Move-construct a `std::string` from `src` into `dst`.
///
/// libc++'s move constructor copies the representation and then default-initialises
/// the source. Zeroing the source's 24 bytes is exactly a default-initialised short
/// string: `__is_long_ = 0`, `__size_ = 0`, `__data_[0] = 0`. Critically it also means
/// the moved-from string owns nothing, so the caller's destructor will not free the
/// buffer this port just took.
///
/// The object file's undefined `_memset` is the C++ doing the same thing.
#[inline]
unsafe fn string_move_construct(dst: *mut StdString, src: *mut StdString) {
    core::ptr::copy_nonoverlapping(src as *const u8, dst as *mut u8, 24);
    core::ptr::write_bytes(src as *mut u8, 0, 24);
}

/// Shared body of the `C1` (complete object) and `C2` (base object) constructor
/// variants. The Itanium ABI emits both even when they are identical; defining only
/// one links until some caller references the other.
#[inline]
unsafe fn error_info_construct(this: *mut ErrorInfo, code: i32, reason: *mut StdString) {
    // m_refs(0) — a plain store, not an atomic RMW: the object is not yet shared.
    (*this).m_refs = AtomicU32::new(0);
    (*this).m_code = code;
    string_move_construct(core::ptr::addr_of_mut!((*this).m_reason), reason);
}

/// `realm::Status::ErrorInfo::ErrorInfo(ErrorCodes::Error, std::string&&)` — C1.
#[export_name = "_ZN5realm6Status9ErrorInfoC1ENS_10ErrorCodes5ErrorEONSt3__112basic_stringIcNS4_11char_traitsIcEENS4_9allocatorIcEEEE"]
pub unsafe extern "C" fn error_info_ctor_c1(this: *mut ErrorInfo, code: i32, reason: *mut StdString) {
    error_info_construct(this, code, reason);
}

/// `realm::Status::ErrorInfo::ErrorInfo(ErrorCodes::Error, std::string&&)` — C2.
#[export_name = "_ZN5realm6Status9ErrorInfoC2ENS_10ErrorCodes5ErrorEONSt3__112basic_stringIcNS4_11char_traitsIcEENS4_9allocatorIcEEEE"]
pub unsafe extern "C" fn error_info_ctor_c2(this: *mut ErrorInfo, code: i32, reason: *mut StdString) {
    error_info_construct(this, code, reason);
}

/// `realm::Status::ErrorInfo::create(ErrorCodes::Error, std::string&&)`
///
/// `return util::bind_ptr<ErrorInfo>(new ErrorInfo(code, std::move(reason)));`
///
/// # This returns via `sret`, and getting that wrong is what broke it first
///
/// `bind_ptr` is one pointer, so "returned in `rax`" is the natural assumption and it
/// is **wrong**. It has a user-provided destructor (`~bind_ptr() { unbind(); }`) and a
/// user-provided copy constructor, which under the Itanium ABI make it **MEMORY class**
/// regardless of size: the caller passes a hidden result pointer in `rdi` and the callee
/// returns that same pointer in `rax`.
///
/// The oracle's own prologue says so unambiguously:
///
/// ```text
/// 6a: movq %rdx, %rbx     ; reason  = 3rd argument
/// 6d: movl %esi, %r14d    ; code    = 2nd argument
/// 70: movq %rdi, %r15     ; sret    = 1st argument
/// 73: movl $0x20, %edi    ; operator new(32)
/// ```
///
/// Declared without the out-parameter, every argument is shifted by one and `reason`
/// receives the value of `code` — a small integer dereferenced as a `std::string*`.
/// That segfaulted on every corpus file the oracle rejects, and `make diff-test` stayed
/// green throughout because no trace builds an error `Status`. `make format-compat`
/// caught it.
///
/// **Triviality, not size, decides the return class.** Any C++ type with a
/// user-provided destructor, copy constructor or move constructor comes back through
/// memory.
///
/// `bind_ptr`'s explicit `T*` constructor **binds**, so the result carries a reference
/// count of 1 — `m_refs` starts at 0 in the constructor and is incremented once here.
/// Too high leaks, too low double-frees, and neither is a byte diff.
///
/// The `REALM_ASSERT(code != ErrorCodes::OK)` is a no-op in this build.
#[export_name = "_ZN5realm6Status9ErrorInfo6createENS_10ErrorCodes5ErrorEONSt3__112basic_stringIcNS4_11char_traitsIcEENS4_9allocatorIcEEEE"]
pub unsafe extern "C-unwind" fn error_info_create(
    out: *mut BindPtr,
    code: i32,
    reason: *mut StdString,
) -> *mut BindPtr {
    let raw = cxx_operator_new(32) as *mut ErrorInfo;
    error_info_construct(raw, code, reason);
    // bind_ptr(T*) -> bind(p) -> p->bind_ptr() -> m_refs.fetch_add(1, relaxed)
    (*raw).m_refs.fetch_add(1, Ordering::Relaxed);
    (*out).m_ptr = raw;
    // SysV x86-64: the callee returns the hidden result pointer in rax.
    out
}

/// `realm::operator<<(std::ostream&, const realm::Status&)`
///
/// `out << val.code_string() << ": " << val.reason();`
///
/// Both accessors are inline in `status.hpp` and are reimplemented here:
/// - `code()` is `m_error ? m_error->m_code : ErrorCodes::OK`
/// - `code_string()` is `ErrorCodes::error_string(code())`
/// - `reason()` is `m_error ? m_error->m_reason : <a static empty string>`
///
/// The static is `Status::reason() const::empty`, a constant-initialised empty
/// `std::string` emitted `weak private external` in every TU that uses `reason()` —
/// two definers in `librealm.a`, so removing this one orphans nothing. For the null
/// case this port streams a zero-length sequence instead of reading that object, which
/// is the same two arguments (`data`, `0`) reaching `__put_character_sequence`.
#[export_name = "_ZN5realmlsERNSt3__113basic_ostreamIcNS0_11char_traitsIcEEEERKNS_6StatusE"]
pub unsafe extern "C-unwind" fn operator_shl_status(os: *mut c_void, val: *const Status) -> *mut c_void {
    let info = (*val).m_error.m_ptr;

    let code = if info.is_null() { 0 } else { (*info).m_code };
    let cs = error_codes_error_string(code);
    let mut out = put_character_sequence(os, cs.data, cs.size);

    const SEP: &str = ": ";
    out = put_character_sequence(out, SEP.as_ptr() as *const c_char, SEP.len());

    if info.is_null() {
        // The static empty string: size 0. Its data pointer is never dereferenced for
        // a zero-length write, so a dangling-but-unused pointer is not passed on;
        // an empty short string's data is its own first byte.
        let empty: [u8; 1] = [0];
        put_character_sequence(out, empty.as_ptr() as *const c_char, 0)
    } else {
        let (data, size) = (*info).m_reason.parts();
        put_character_sequence(out, data, size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn short_string(s: &str) -> StdString {
        assert!(s.len() <= 22);
        let mut rep = [0u8; 24];
        rep[0] = (s.len() as u8) << 1; // is_long = 0, size in bits 1..7
        rep[1..1 + s.len()].copy_from_slice(s.as_bytes());
        StdString { rep }
    }

    #[test]
    fn reads_the_short_representation() {
        let s = short_string("hello");
        let (data, size) = unsafe { s.parts() };
        assert_eq!(size, 5);
        let bytes = unsafe { core::slice::from_raw_parts(data as *const u8, size) };
        assert_eq!(bytes, b"hello");
    }

    #[test]
    fn reads_the_empty_short_representation() {
        let s = StdString { rep: [0u8; 24] };
        let (_, size) = unsafe { s.parts() };
        assert_eq!(size, 0);
    }

    #[test]
    fn reads_the_long_representation() {
        // is_long in the LOW bit — the whole point of the layout note above.
        let payload = b"a long string past the short buffer\0";
        let mut rep = [0u8; 24];
        let cap: usize = 64;
        rep[0..8].copy_from_slice(&((cap << 1) | 1).to_ne_bytes());
        rep[8..16].copy_from_slice(&(35usize).to_ne_bytes());
        rep[16..24].copy_from_slice(&(payload.as_ptr() as usize).to_ne_bytes());
        let s = StdString { rep };
        let (data, size) = unsafe { s.parts() };
        assert_eq!(size, 35);
        assert_eq!(data as usize, payload.as_ptr() as usize);
    }

    #[test]
    fn move_construct_empties_the_source() {
        let mut src = short_string("moved");
        let mut dst = StdString { rep: [0xAA; 24] };
        unsafe { string_move_construct(&mut dst, &mut src) };
        let (_, dsize) = unsafe { dst.parts() };
        assert_eq!(dsize, 5);
        // The source must be a valid EMPTY string, or the caller's destructor frees a
        // buffer this port now owns.
        assert_eq!(src.rep, [0u8; 24]);
        let (_, ssize) = unsafe { src.parts() };
        assert_eq!(ssize, 0);
    }
}
