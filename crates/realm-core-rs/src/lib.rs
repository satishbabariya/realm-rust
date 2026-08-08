//! Rust replacements for realm-core translation units.
//!
//! Every item here is linked into the *hybrid* stack only. The oracle keeps using
//! the original C++, and `make diff-test` compares the `.realm` files the two stacks
//! produce. Byte-identity is the acceptance criterion — see CLAUDE.md.
//!
//! Nothing has been ported yet. That is why the hybrid is currently byte-identical
//! to the oracle by construction, and why a green `make diff-test` right now proves
//! the harness works rather than that any Rust is correct.
//!
//! When adding a unit:
//!   - `#[no_mangle] pub extern "C"` with the exact symbol the hybrid link needs
//!   - `#[repr(C)]` on anything crossing the boundary or reaching the file
//!   - mirror the C++ width/alignment arithmetic rather than re-deriving it
//!     (see .claude/rules/format-fidelity.md)

#![deny(improper_ctypes_definitions)]

/// Presence probe for the hybrid build.
///
/// The harness calls nothing here yet, but a staticlib with no exported symbols is
/// liable to be dropped entirely by the linker, which would make "the Rust is linked"
/// and "the Rust is absent" indistinguishable — exactly the ambiguity this project
/// cannot afford.
#[no_mangle]
pub extern "C" fn realm_rs_units_ported() -> u32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_reports_zero_until_a_unit_lands() {
        assert_eq!(realm_rs_units_ported(), 0);
    }
}
