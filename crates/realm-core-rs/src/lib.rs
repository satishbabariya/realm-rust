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
pub mod array_unsigned;
pub mod disable_sync_to_disk;
pub mod error_codes;
mod error_codes_table;
pub mod object_id;
pub mod status;
pub mod string_data;
pub mod utilities;
pub mod util;

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
    11
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
