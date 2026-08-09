//! Port of `upstream/src/realm/error_codes.cpp`.
//!
//! Reachable but **byte-invisible**: error names and categories are diagnostics and
//! never reach a `.realm`. `make verify` therefore proves the link is intact and
//! nothing else regressed; the evidence that this port is correct is
//! `migration/checks/run_error_codes_differential.sh`, which checks every one of the
//! 160 table rows, every category, and the `from_string` search semantics against the
//! oracle.
//!
//! # The tables are generated from the oracle
//!
//! See `error_codes_table.rs`. Two lookup tables — a 160-entry name→code array and a
//! ~400-line `switch` over categories — were extracted by linking a generator against
//! `librealm.a`, not re-typed from the C++. That is stated openly there because it
//! means this port cannot independently confirm the table is *right*, only that it is
//! *the same*, which is the property byte-identity needs.
//!
//! Completeness of the category switch was established by probing every code in
//! `0..=4_000_000` (largest table code: 1_000_000) plus negatives and `INT_MAX`:
//! exactly two codes outside the name table have non-default categories.
//!
//! # ABI, measured
//!
//! | C++ type | layout | passing |
//! |---|---|---|
//! | `ErrorCodes::Error` | `int` | 4 bytes, `edi`/`eax` |
//! | `ErrorCategory` | one `unsigned m_value` | 4 bytes, returned in `eax` |
//! | `std::string_view` | `{const char*, size_t}` | 16 B, 2 GPRs |
//! | `std::pair<string_view, Error>` | `first` @0, `second` @16, `sizeof = 24` | — |
//! | `std::vector<T>` | `{begin, end, cap}` | 24 B, **sret** |
//!
//! All three `get_*` functions return a `std::vector` by value that C++ then destroys,
//! so the buffers come from `operator new` and the capacity has to match what
//! `push_back` would have produced — see `push_back_capacity`.

use core::ffi::{c_char, c_void};

use crate::error_codes_table::{EXTRA_CATEGORIES, STRING_TO_ERROR_CODE};

/// `ErrorCodes::UnknownError`. Not present in the name table — `error_string` reports
/// "unknown" for it and its category is 0 — so it is written out rather than looked up.
const UNKNOWN_ERROR: i32 = 2_000_000;

/// `std::string_view`.
#[repr(C)]
pub struct StringView {
    data: *const c_char,
    size: usize,
}

/// `realm::ErrorCategory` — a single `unsigned`, so it returns in `eax` exactly as a
/// bare `u32` would. Kept as a struct because it crosses the boundary.
#[repr(C)]
pub struct ErrorCategory {
    m_value: u32,
}

/// `std::pair<std::string_view, ErrorCodes::Error>`. The 4 tail padding bytes are
/// zeroed here; C++ leaves them indeterminate. Nothing reads them, and a deterministic
/// value is preferable to reproducing "indeterminate".
#[repr(C)]
struct PairSvError {
    first: StringView,
    second: i32,
    _pad: u32,
}

/// `std::vector<T>` under libc++: three pointers, no small-buffer optimisation.
#[repr(C)]
pub struct VectorRaw {
    begin: *mut u8,
    end: *mut u8,
    cap: *mut u8,
}

const _: () = {
    assert!(core::mem::size_of::<StringView>() == 16);
    assert!(core::mem::size_of::<ErrorCategory>() == 4);
    assert!(core::mem::size_of::<PairSvError>() == 24);
    assert!(core::mem::offset_of!(PairSvError, second) == 16);
    assert!(core::mem::size_of::<VectorRaw>() == 24);
};

extern "C-unwind" {
    /// `operator new(unsigned long)`. Throws `std::bad_alloc`.
    ///
    /// Declared here as well as in `util::base64` rather than shared: two `extern`
    /// declarations of the same symbol are harmless, and a shared `cxx_abi` module is
    /// a refactor that should be done once for all units, not smuggled into a port.
    #[link_name = "_Znwm"]
    fn cxx_operator_new(size: usize) -> *mut u8;

    /// `std::__put_character_sequence<char, char_traits<char>>(ostream&, const char*, size_t)`
    ///
    /// What `operator<<(ostream&, string_view)` inlines to in libc++. It is
    /// `weak private external`, which is linkable — see the `array_unsigned` entry in
    /// `migration/JOURNAL.md` for the experiment that established that.
    #[link_name = "_ZNSt3__124__put_character_sequenceB9nqe210106IcNS_11char_traitsIcEEEERNS_13basic_ostreamIT_T0_EES7_PKS4_m"]
    fn put_character_sequence(os: *mut c_void, s: *const c_char, len: usize) -> *mut c_void;
}

/// Capacity a libc++ `std::vector<T>` ends up with after `n` `push_back`s from empty.
///
/// Not `n.next_power_of_two()`: that happens to agree for `n = 160` but is a different
/// function, and the table size is not fixed by anything. This replays libc++'s
/// `__recommend(size + 1) = max(2 * capacity, size + 1)` growth, which is the rule the
/// C++ actually followed.
fn push_back_capacity(n: usize) -> usize {
    let mut cap = 0usize;
    let mut size = 0usize;
    while size < n {
        if size == cap {
            let doubled = 2 * cap;
            cap = if doubled > size + 1 { doubled } else { size + 1 };
        }
        size += 1;
    }
    cap
}

/// Allocate a vector buffer of `cap` elements and return `{begin, end, cap}` with
/// `end` set for `len` elements. `cap == 0` yields three null pointers, which is what
/// an unallocated libc++ vector looks like — not a zero-size allocation.
unsafe fn new_vector(elem_size: usize, len: usize, cap: usize) -> (VectorRaw, *mut u8) {
    if cap == 0 {
        return (
            VectorRaw {
                begin: core::ptr::null_mut(),
                end: core::ptr::null_mut(),
                cap: core::ptr::null_mut(),
            },
            core::ptr::null_mut(),
        );
    }
    let begin = cxx_operator_new(cap * elem_size);
    (
        VectorRaw {
            begin,
            end: begin.add(len * elem_size),
            cap: begin.add(cap * elem_size),
        },
        begin,
    )
}

/// `strncmp(a, b, b.len())` where `a` is a NUL-terminated C string.
///
/// The C++ comparator is
/// `strncmp(ec_pair.name, name.data(), name.size()) < 0`, so the comparison length is
/// the *needle's* length, not the entry's. That is what makes a prefix of an entry
/// compare equal here and get rejected afterwards by the exact `==` check.
///
/// `strncmp` compares as `unsigned char` and stops at the first NUL in either operand.
/// Mirrored exactly, including never reading past the entry's implicit NUL.
fn strncmp_c(entry: &str, needle: &[u8]) -> i32 {
    let a = entry.as_bytes();
    for i in 0..needle.len() {
        let ca = if i < a.len() { a[i] } else { 0u8 };
        let cb = needle[i];
        if ca != cb {
            return ca as i32 - cb as i32;
        }
        if ca == 0 {
            return 0;
        }
    }
    0
}

// ===========================================================================
// Exported symbols.
// ===========================================================================

/// `realm::ErrorCodes::error_categories(Error)`
#[export_name = "_ZN5realm10ErrorCodes16error_categoriesENS0_5ErrorE"]
pub extern "C" fn error_categories(code: i32) -> ErrorCategory {
    for &(_, c, cat) in STRING_TO_ERROR_CODE.iter() {
        if c == code {
            return ErrorCategory { m_value: cat };
        }
    }
    for &(c, cat) in EXTRA_CATEGORIES.iter() {
        if c == code {
            return ErrorCategory { m_value: cat };
        }
    }
    // The C++ switch's default falls through to a default-constructed ErrorCategory.
    ErrorCategory { m_value: 0 }
}

/// `realm::ErrorCodes::error_string(Error)`
///
/// A linear scan in the C++ (`for (auto [name, c] : string_to_error_code)`), returning
/// the **first** match. The table has no duplicate codes — checked against the oracle —
/// so first-match and any-match agree, but the scan order is mirrored regardless.
#[export_name = "_ZN5realm10ErrorCodes12error_stringENS0_5ErrorE"]
pub extern "C" fn error_string(code: i32) -> StringView {
    for &(name, c, _) in STRING_TO_ERROR_CODE.iter() {
        if code == c {
            return StringView {
                data: name.as_ptr() as *const c_char,
                size: name.len(),
            };
        }
    }
    const UNKNOWN: &str = "unknown";
    StringView {
        data: UNKNOWN.as_ptr() as *const c_char,
        size: UNKNOWN.len(),
    }
}

/// `realm::ErrorCodes::from_string(std::string_view)`
///
/// `std::lower_bound` over the name-sorted table with the `strncmp` comparator above,
/// then an exact `string_view` equality check. Both halves are load-bearing: the
/// comparator only inspects `needle.len()` bytes, so `"AWS"` lands on `"AWSError"` and
/// is rejected only by the equality check.
#[export_name = "_ZN5realm10ErrorCodes11from_stringENSt3__117basic_string_viewIcNS1_11char_traitsIcEEEE"]
pub unsafe extern "C" fn from_string(name: StringView) -> i32 {
    let needle: &[u8] = if name.size == 0 {
        &[]
    } else {
        core::slice::from_raw_parts(name.data as *const u8, name.size)
    };

    // std::lower_bound: first element for which !comp(element, value).
    let mut low = 0usize;
    let mut count = STRING_TO_ERROR_CODE.len();
    while count > 0 {
        let half = count / 2;
        let probe = low + half;
        if strncmp_c(STRING_TO_ERROR_CODE[probe].0, needle) < 0 {
            low = probe + 1;
            count -= half + 1;
        } else {
            count = half;
        }
    }

    if low != STRING_TO_ERROR_CODE.len() {
        let (entry_name, code, _) = STRING_TO_ERROR_CODE[low];
        // `it->name == name`: string_view equality, i.e. same length and same bytes.
        if entry_name.as_bytes() == needle {
            return code;
        }
    }
    UNKNOWN_ERROR
}

/// `realm::ErrorCodes::get_all_codes()` → `std::vector<Error>`
#[export_name = "_ZN5realm10ErrorCodes13get_all_codesEv"]
pub unsafe extern "C-unwind" fn get_all_codes() -> VectorRaw {
    let n = STRING_TO_ERROR_CODE.len();
    let (vec, begin) = new_vector(4, n, push_back_capacity(n));
    let p = begin as *mut i32;
    for (i, &(_, code, _)) in STRING_TO_ERROR_CODE.iter().enumerate() {
        p.add(i).write(code);
    }
    vec
}

/// `realm::ErrorCodes::get_all_names()` → `std::vector<std::string_view>`
#[export_name = "_ZN5realm10ErrorCodes13get_all_namesEv"]
pub unsafe extern "C-unwind" fn get_all_names() -> VectorRaw {
    let n = STRING_TO_ERROR_CODE.len();
    let (vec, begin) = new_vector(16, n, push_back_capacity(n));
    let p = begin as *mut StringView;
    for (i, &(name, _, _)) in STRING_TO_ERROR_CODE.iter().enumerate() {
        p.add(i).write(StringView {
            data: name.as_ptr() as *const c_char,
            size: name.len(),
        });
    }
    vec
}

/// `realm::ErrorCodes::get_error_list()` → `std::vector<std::pair<std::string_view, Error>>`
#[export_name = "_ZN5realm10ErrorCodes14get_error_listEv"]
pub unsafe extern "C-unwind" fn get_error_list() -> VectorRaw {
    let n = STRING_TO_ERROR_CODE.len();
    let (vec, begin) = new_vector(24, n, push_back_capacity(n));
    let p = begin as *mut PairSvError;
    for (i, &(name, code, _)) in STRING_TO_ERROR_CODE.iter().enumerate() {
        p.add(i).write(PairSvError {
            first: StringView {
                data: name.as_ptr() as *const c_char,
                size: name.len(),
            },
            second: code,
            _pad: 0,
        });
    }
    vec
}

/// `realm::operator<<(std::ostream&, ErrorCodes::Error)`
///
/// `return stream << ErrorCodes::error_string(code);` — and
/// `operator<<(ostream&, string_view)` in libc++ is exactly
/// `__put_character_sequence(os, sv.data(), sv.size())`.
///
/// `C-unwind` because the stream can set badbit and rethrow.
#[export_name = "_ZN5realmlsERNSt3__113basic_ostreamIcNS0_11char_traitsIcEEEENS_10ErrorCodes5ErrorE"]
pub unsafe extern "C-unwind" fn operator_shl(os: *mut c_void, code: i32) -> *mut c_void {
    let sv = error_string(code);
    put_character_sequence(os, sv.data, sv.size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_sorted_by_name() {
        // from_string's lower_bound depends on it, and the C++ has a debug-only
        // is_sorted assertion that is compiled out in this build.
        for w in STRING_TO_ERROR_CODE.windows(2) {
            assert!(w[0].0 < w[1].0, "{} !< {}", w[0].0, w[1].0);
        }
    }

    #[test]
    fn no_duplicate_codes() {
        // error_string returns the first match; duplicates would make scan order
        // observable.
        let mut codes: Vec<i32> = STRING_TO_ERROR_CODE.iter().map(|e| e.1).collect();
        codes.sort_unstable();
        let before = codes.len();
        codes.dedup();
        assert_eq!(before, codes.len());
    }

    #[test]
    fn push_back_capacity_replays_libcxx_growth() {
        assert_eq!(push_back_capacity(0), 0);
        assert_eq!(push_back_capacity(1), 1);
        assert_eq!(push_back_capacity(2), 2);
        assert_eq!(push_back_capacity(3), 4);
        assert_eq!(push_back_capacity(5), 8);
        // The value the oracle actually reported for all three get_* functions.
        assert_eq!(push_back_capacity(160), 256);
    }

    #[test]
    fn strncmp_matches_c_semantics() {
        assert_eq!(strncmp_c("AWSError", b"AWSError"), 0);
        // Needle shorter than the entry: only needle.len() bytes are compared, so a
        // prefix compares EQUAL here. This is why from_string needs the second check.
        assert_eq!(strncmp_c("AWSError", b"AWS"), 0);
        // Needle longer: the entry's implicit NUL loses to any real byte.
        assert!(strncmp_c("AWSError", b"AWSErrorX") < 0);
        assert!(strncmp_c("B", b"A") > 0);
        assert_eq!(strncmp_c("anything", b""), 0);
    }

    #[test]
    fn from_string_rejects_prefixes_and_extensions() {
        let f = |s: &str| unsafe {
            from_string(StringView {
                data: s.as_ptr() as *const c_char,
                size: s.len(),
            })
        };
        // Values confirmed against the oracle.
        assert_eq!(f("AWSError"), 4310);
        assert_eq!(f("OK"), 0);
        assert_eq!(f("WrongThread"), 2006);
        assert_eq!(f("AWS"), UNKNOWN_ERROR);
        assert_eq!(f("AWSErrorX"), UNKNOWN_ERROR);
        assert_eq!(f(""), UNKNOWN_ERROR);
        assert_eq!(f("zzz"), UNKNOWN_ERROR);
    }

    #[test]
    fn unknown_error_is_not_in_the_table() {
        assert!(STRING_TO_ERROR_CODE.iter().all(|e| e.1 != UNKNOWN_ERROR));
        assert_eq!(error_categories(UNKNOWN_ERROR).m_value, 0);
    }

    #[test]
    fn extra_categories_have_no_name() {
        for &(code, cat) in EXTRA_CATEGORIES.iter() {
            assert!(STRING_TO_ERROR_CODE.iter().all(|e| e.1 != code));
            assert_eq!(error_categories(code).m_value, cat);
        }
    }
}
