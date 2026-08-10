// Can Rust throw a realm::LogicError that C++ catches?
// exceptions.cpp stays C++, so the typeinfo, vtable, ctor and dtor all already exist in
// the link. If this works, "throwing" needs ~20 lines of glue, not an RTTI shim.
#![crate_type = "staticlib"]
use core::ffi::{c_char, c_int, c_void};

#[repr(C)]
struct StringView { data: *const c_char, size: usize }

extern "C" {
    fn __cxa_allocate_exception(size: usize) -> *mut c_void;
    #[link_name = "_ZN5realm10LogicErrorC1ENS_10ErrorCodes5ErrorENSt3__117basic_string_viewIcNS3_11char_traitsIcEEEE"]
    fn logic_error_ctor(this: *mut c_void, code: c_int, msg: StringView);
    #[link_name = "_ZN5realm10LogicErrorD1Ev"]
    fn logic_error_dtor(this: *mut c_void);
    #[link_name = "_ZTIN5realm10LogicErrorE"]
    static LOGIC_ERROR_TYPEINFO: c_void;
}
extern "C-unwind" {
    fn __cxa_throw(exc: *mut c_void, tinfo: *const c_void, dest: unsafe extern "C" fn(*mut c_void)) -> !;
}

/// ErrorCodes::WrongTransactionState
const WRONG_TRANSACTION_STATE: c_int = 1015;

#[no_mangle]
pub unsafe extern "C-unwind" fn rust_throws_logic_error(code: c_int, msg: *const c_char, len: usize) {
    let e = __cxa_allocate_exception(16);
    logic_error_ctor(e, code, StringView { data: msg, size: len });
    __cxa_throw(e, &LOGIC_ERROR_TYPEINFO as *const c_void, logic_error_dtor);
}

#[no_mangle]
pub extern "C" fn rust_probe_present() -> c_int { WRONG_TRANSACTION_STATE }
