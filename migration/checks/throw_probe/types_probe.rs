// Can Rust throw each of the four exception types `util/file_mapper.cpp` throws, such
// that C++ catches each AS ITS OWN TYPE?
//
// Catching as a base class would pass even with a wrong vptr or a wrong typeinfo, which
// is exactly the mistake this probe exists to rule out -- AddressSpaceExhausted has to be
// built by calling its RuntimeError base constructor and then overwriting the vptr, and
// nothing but a derived-type catch can tell whether step two worked.
//
// Built with panic=unwind; see Cargo.toml and lib.rs for why that is safe here.
#![crate_type = "staticlib"]
use core::ffi::{c_char, c_int, c_void};

#[repr(C)]
struct StringView {
    data: *const c_char,
    size: usize,
}

#[repr(C)]
struct StdString {
    rep: [u8; 24],
}

extern "C" {
    fn __cxa_allocate_exception(size: usize) -> *mut c_void;

    // --- realm ---
    #[link_name = "_ZN5realm12RuntimeErrorC1ENS_10ErrorCodes5ErrorENSt3__117basic_string_viewIcNS3_11char_traitsIcEEEE"]
    fn runtime_error_ctor(this: *mut c_void, code: c_int, msg: StringView);
    #[link_name = "_ZN5realm21AddressSpaceExhaustedD1Ev"]
    fn ase_dtor(this: *mut c_void);
    #[link_name = "_ZTVN5realm21AddressSpaceExhaustedE"]
    static ASE_VTABLE: c_void;
    #[link_name = "_ZTIN5realm21AddressSpaceExhaustedE"]
    static ASE_TYPEINFO: c_void;

    #[link_name = "_ZN5realm11SystemErrorC1EiNSt3__117basic_string_viewIcNS1_11char_traitsIcEEEE"]
    fn system_error_realm_ctor(this: *mut c_void, err: c_int, msg: StringView);
    #[link_name = "_ZN5realm11SystemErrorD1Ev"]
    fn system_error_realm_dtor(this: *mut c_void);
    #[link_name = "_ZTIN5realm11SystemErrorE"]
    static REALM_SYSTEM_ERROR_TYPEINFO: c_void;

    // --- libc++ ---
    #[link_name = "_ZNSt3__112system_errorC1EiRKNS_14error_categoryERKNS_12basic_stringIcNS_11char_traitsIcEENS_9allocatorIcEEEE"]
    fn std_system_error_ctor(this: *mut c_void, ev: c_int, cat: *const c_void, what: *const StdString);
    #[link_name = "_ZNSt3__112system_errorD1Ev"]
    fn std_system_error_dtor(this: *mut c_void);
    #[link_name = "_ZTINSt3__112system_errorE"]
    static STD_SYSTEM_ERROR_TYPEINFO: c_void;
    #[link_name = "_ZNSt3__115system_categoryEv"]
    fn std_system_category() -> *const c_void;

    #[link_name = "_ZNSt13runtime_errorC1ERKNSt3__112basic_stringIcNS0_11char_traitsIcEENS0_9allocatorIcEEEE"]
    fn std_runtime_error_ctor(this: *mut c_void, what: *const StdString);
    #[link_name = "_ZNSt13runtime_errorD1Ev"]
    fn std_runtime_error_dtor(this: *mut c_void);
    #[link_name = "_ZTISt13runtime_error"]
    static STD_RUNTIME_ERROR_TYPEINFO: c_void;

    #[link_name = "_ZNSt3__112basic_stringIcNS_11char_traitsIcEENS_9allocatorIcEEE6appendEPKcm"]
    fn string_append(this: *mut StdString, s: *const c_char, n: usize) -> *mut StdString;
    #[link_name = "_ZNSt3__112basic_stringIcNS_11char_traitsIcEENS_9allocatorIcEEED1Ev"]
    fn string_dtor(this: *mut StdString);
}

extern "C-unwind" {
    fn __cxa_throw(exc: *mut c_void, tinfo: *const c_void, dest: unsafe extern "C" fn(*mut c_void)) -> !;
}

const ERRORCODES_ADDRESS_SPACE_EXHAUSTED: c_int = 1005;

unsafe fn make_string(s: &str) -> StdString {
    let mut out = StdString { rep: [0u8; 24] };
    string_append(&mut out, s.as_ptr() as *const c_char, s.len());
    out
}

/// `throw realm::AddressSpaceExhausted(msg)`.
///
/// Its constructor is **inline**, so there is no symbol to call. Build it by running the
/// bindable `RuntimeError` base constructor and then overwriting the vptr with this
/// class's own vtable + 16 (past the offset-to-top and typeinfo slots).
#[no_mangle]
pub unsafe extern "C-unwind" fn throw_address_space_exhausted(msg: *const c_char, len: usize) {
    let e = __cxa_allocate_exception(16);
    runtime_error_ctor(
        e,
        ERRORCODES_ADDRESS_SPACE_EXHAUSTED,
        StringView { data: msg, size: len },
    );
    let vptr = (&ASE_VTABLE as *const c_void as *const u8).add(16);
    *(e as *mut *const u8) = vptr;
    __cxa_throw(e, &ASE_TYPEINFO as *const c_void, ase_dtor)
}

/// `throw realm::SystemError(err, msg)` — constructor is out-of-line, so no vptr fixup.
#[no_mangle]
pub unsafe extern "C-unwind" fn throw_realm_system_error(err: c_int, msg: *const c_char, len: usize) {
    let e = __cxa_allocate_exception(16);
    system_error_realm_ctor(e, err, StringView { data: msg, size: len });
    __cxa_throw(e, &REALM_SYSTEM_ERROR_TYPEINFO as *const c_void, system_error_realm_dtor)
}

/// `throw std::system_error(err, std::system_category(), what)`.
#[no_mangle]
pub unsafe extern "C-unwind" fn throw_std_system_error(err: c_int, msg: *const c_char, len: usize) {
    let mut what = make_string(core::str::from_utf8_unchecked(core::slice::from_raw_parts(
        msg as *const u8,
        len,
    )));
    let e = __cxa_allocate_exception(32);
    std_system_error_ctor(e, err, std_system_category(), &what);
    string_dtor(&mut what);
    __cxa_throw(e, &STD_SYSTEM_ERROR_TYPEINFO as *const c_void, std_system_error_dtor)
}

/// `throw std::runtime_error(what)`.
#[no_mangle]
pub unsafe extern "C-unwind" fn throw_std_runtime_error(msg: *const c_char, len: usize) {
    let mut what = make_string(core::str::from_utf8_unchecked(core::slice::from_raw_parts(
        msg as *const u8,
        len,
    )));
    let e = __cxa_allocate_exception(16);
    std_runtime_error_ctor(e, &what);
    string_dtor(&mut what);
    __cxa_throw(e, &STD_RUNTIME_ERROR_TYPEINFO as *const c_void, std_runtime_error_dtor)
}
