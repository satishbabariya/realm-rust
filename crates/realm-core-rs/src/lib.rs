//! Rust replacements for realm-core translation units.
//!
//! Every item here is linked into the *hybrid* stack only. The oracle keeps using
//! the original C++, and `make diff-test` compares the `.realm` files the two stacks
//! produce. Byte-identity is the acceptance criterion — see CLAUDE.md.
//!
//! The C++ translation units these definitions replace are listed in
//! `ported_units.txt` next to this crate's manifest, and `harness/CMakeLists.txt`
//! removes exactly those from the hybrid's `Storage` target. Adding a unit here
//! without adding it there leaves the C++ definition in the link and the Rust
//! unreferenced — which looks identical to a successful port from the outside.
//!
//! When adding a unit:
//!   - `#[export_name = "<exact mangled symbol>"]` on a `pub extern "C"` fn
//!   - `#[repr(C)]` on anything crossing the boundary or reaching the file
//!   - mirror the C++ width/alignment arithmetic rather than re-deriving it
//!     (see .claude/rules/format-fidelity.md)
//!   - add the unit to `ported_units.txt`

#![deny(improper_ctypes_definitions)]

pub mod array_blob;
pub mod array_blobs_small;
pub mod array_timestamp;
pub mod array_unsigned;
pub mod decimal128;
pub mod disable_sync_to_disk;
pub mod error_codes;
mod error_codes_table;
pub mod object_id;
pub mod status;
pub mod string_data;
pub mod utilities;
pub mod unicode;
pub mod util;

// ---------------------------------------------------------------------------
// Abort on panic, while still allowing C++ exceptions to unwind through Rust.
// ---------------------------------------------------------------------------

/// The workspace builds with `panic = "unwind"`, which it has to: `Allocator::alloc` is
/// inline in `alloc.hpp` and throws `LogicError` on a read-only allocator, so every unit
/// that allocates a node inherits a throw site. Under `panic = "abort"` that unwind hits
/// the Rust frame and aborts — `migration/checks/throw_probe/` demonstrates it.
///
/// Unwinding alone would give up something this project cannot afford. Today an
/// `overflow-checks` trap or an index panic in Rust dies at the point of failure. Left
/// unwinding, that same panic travels into C++, where any `catch (...)` up the stack can
/// swallow it and continue with corrupt state — the exact silent-corruption failure mode
/// the whole harness exists to catch.
///
/// **A panic hook restores it.** The hook runs *before* unwinding begins, so aborting
/// inside it reproduces `panic = "abort"` behaviour for Rust panics — message printed,
/// process dead at the failure site. Deliberate C++ exceptions are not Rust panics and
/// never reach the hook.
///
/// This is one registration rather than a `catch_unwind` at each of the 125 exported
/// functions, and that is the point: a barrier per export is 125 chances to forget one,
/// and the next export added would silently lack it. A hook cannot be forgotten.
/// `catch_unwind` also cannot be used at a boundary that *deliberately* throws — catching
/// a foreign exception with it aborts — so the per-export form would need an exception
/// list exactly where the risk is highest.
fn abort_on_panic(info: &std::panic::PanicHookInfo<'_>) {
    // Print through the default machinery first so the message and location survive.
    eprintln!("realm-core-rs: {info}");
    std::process::abort();
}

/// Installed at load time via a module initialiser, because a staticlib linked into C++
/// has no other entry point — `main` belongs to the host and nothing calls into Rust
/// before the first ported symbol runs.
extern "C" fn install_abort_on_panic() {
    std::panic::set_hook(Box::new(abort_on_panic));
}

#[cfg(target_vendor = "apple")]
#[used]
#[link_section = "__DATA,__mod_init_func"]
static INSTALL_ABORT_ON_PANIC: extern "C" fn() = install_abort_on_panic;

#[cfg(all(unix, not(target_vendor = "apple")))]
#[used]
#[link_section = ".init_array"]
static INSTALL_ABORT_ON_PANIC: extern "C" fn() = install_abort_on_panic;

/// Presence probe for the hybrid build.
///
/// The harness calls nothing here yet, but a staticlib with no exported symbols is
/// liable to be dropped entirely by the linker, which would make "the Rust is linked"
/// and "the Rust is absent" indistinguishable — exactly the ambiguity this project
/// cannot afford.
///
/// The count is the number of translation units served from Rust. It must equal the
/// number of entries in `ported_units.txt`; `tests::probe_matches_the_manifest`
/// enforces that, so the two cannot drift.
#[no_mangle]
pub extern "C" fn realm_rs_units_ported() -> u32 {
    15
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_matches_the_manifest() {
        let manifest = include_str!("../ported_units.txt");
        let units = manifest
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .count();
        assert_eq!(
            realm_rs_units_ported() as usize,
            units,
            "the probe and ported_units.txt disagree about how much C++ has been replaced"
        );
    }
}
