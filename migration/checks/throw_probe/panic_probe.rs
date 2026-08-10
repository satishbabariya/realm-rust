// Does the abort-on-panic hook survive `panic = "unwind"` and a C++ `catch (...)`?
//
// Same hook and same installation mechanism as crates/realm-core-rs/src/lib.rs. If this
// ever stops aborting, a Rust bug in the hybrid can be swallowed by C++ and the process
// continues with corrupt state -- which is the failure mode the hook exists to prevent.
#![crate_type = "staticlib"]
use core::ffi::c_int;

fn abort_on_panic(info: &std::panic::PanicHookInfo<'_>) {
    eprintln!("realm-core-rs: {info}");
    std::process::abort();
}
extern "C" fn install_abort_on_panic() {
    std::panic::set_hook(Box::new(abort_on_panic));
}
#[cfg(target_vendor = "apple")]
#[used]
#[link_section = "__DATA,__mod_init_func"]
static INSTALL: extern "C" fn() = install_abort_on_panic;

/// Overflow, exactly as `overflow-checks = true` would trap it in a width calculation.
#[no_mangle]
pub extern "C-unwind" fn rust_panics(x: c_int) -> c_int {
    let v: Vec<c_int> = vec![1, 2, 3];
    v[x as usize] // index panic when x >= 3
}
