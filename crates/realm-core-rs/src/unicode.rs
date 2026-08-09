//! Port of `upstream/src/realm/unicode.cpp`.
//!
//! **Byte-invisible and untraced.** These are query-side string folds — case mapping,
//! case-insensitive compare and LIKE — so nothing here reaches a `.realm`, and no trace
//! calls any of them (measured: 0 hits on `case_map`, `equal_case_fold` and
//! `sequence_length` across all five traces). `make verify` proves the link is intact
//! and nothing else regressed; the evidence is
//! `migration/checks/run_unicode_differential.sh`.
//!
//! The upside of that classification: every function here is **pure**, so the
//! differential can be near-exhaustive over byte space rather than sampled.
//!
//! # `resize` is called, not reproduced
//!
//! `case_map` starts with `result.resize(source.size())`, and libc++'s capacity rule for
//! `resize` on an empty string is *not* the one measured for `object_id`'s
//! `(ptr, n)` constructor:
//!
//! ```text
//! n <= 22        short, capacity 22
//! n in 23..=47   capacity 47   (allocation 48)
//! n >= 48        allocation = round_up(n + 1, 8), capacity = allocation - 1
//! ```
//!
//! That was measured — and then not used, because the object file leaves
//! `std::string::append(size_t, char)` **undefined**, which is what `resize` calls to
//! grow. Binding it gives libc++'s exact behaviour, capacity included, with no formula
//! to get wrong. The measurement is recorded above only so the next unit that has to
//! *construct* a string rather than grow one does not have to redo it.
//!
//! # Mirrors worth naming
//!
//! - `case_map` **preserves byte length**: if a mapped character would encode to a
//!   different number of bytes, the original is kept. That is why only ASCII and the
//!   Latin-1 supplement are folded and 3- and 4-byte sequences are copied through.
//! - `string_like_ins(text, upper, lower)` calls `matchlike_ins(text, lower, upper)`,
//!   passing `lower` into the parameter named `pattern_upper`. The swap is real in the
//!   source and is preserved here — but it is **behaviourally inert**: `matchlike`
//!   compares each pattern character against both foldings, so the two arguments are
//!   interchangeable. Measured, not assumed: a negative control that removes the swap
//!   leaves the differential green. Preserved anyway, because "inert today" is a
//!   property of `matchlike`, not of this call site.
//! - `matchlike_ins`'s `const char*` form goes through `StringData(const char*)`, which
//!   is `strlen`-based, so an embedded NUL truncates. Mirrored.

use core::ffi::{c_char, c_void};

/// `realm::StringData`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct StringData {
    data: *const c_char,
    size: usize,
}

/// `std::string`, 24 opaque bytes. Written only through `append`; read through `parts`.
///
/// `align(8)` is load-bearing: without it the array has alignment 1 and
/// `optional<string>` comes out 25 bytes instead of 32, which the assert below catches.
#[repr(C, align(8))]
pub struct StdString {
    rep: [u8; 24],
}

/// `std::optional<std::string>` — string at 0, engaged flag at 24, `sizeof = 32`.
#[repr(C)]
pub struct OptString {
    s: StdString,
    engaged: bool,
}

const _: () = {
    assert!(core::mem::size_of::<StringData>() == 16);
    assert!(core::mem::size_of::<StdString>() == 24);
    assert!(core::mem::size_of::<OptString>() == 32);
};

extern "C-unwind" {
    /// `std::string::append(size_t, char)` — what `resize` calls to grow.
    #[link_name = "_ZNSt3__112basic_stringIcNS_11char_traitsIcEENS_9allocatorIcEEE6appendEmc"]
    fn string_append(this: *mut StdString, n: usize, c: c_char) -> *mut StdString;
    /// `operator delete(void*)` — the unsized form, as bound by `util::base64`.
    #[link_name = "_ZdlPv"]
    fn cxx_operator_delete(p: *mut u8);
}

extern "C" {
    /// `realm::StringData::matchlike_ins(const StringData&, const StringData&, const StringData&)`
    /// — supplied by this crate's `string_data` port.
    #[link_name = "_ZN5realm10StringData13matchlike_insERKS0_S2_S2_"]
    fn matchlike_ins(text: *const StringData, lower: *const StringData, upper: *const StringData) -> bool;
}

// ---------------------------------------------------------------------------
// std::string helpers. libc++: is_long is the LOW bit of byte 0 (see status.rs).
// ---------------------------------------------------------------------------

impl StdString {
    const EMPTY: StdString = StdString { rep: [0u8; 24] };

    #[inline]
    unsafe fn is_long(&self) -> bool {
        self.rep[0] & 1 != 0
    }

    /// `(data, size)`.
    #[inline]
    unsafe fn parts(&self) -> (*mut c_char, usize) {
        let base = self.rep.as_ptr();
        if !self.is_long() {
            (base.add(1) as *mut c_char, (*base >> 1) as usize)
        } else {
            let w = base as *const usize;
            (w.add(2).read_unaligned() as *mut c_char, w.add(1).read_unaligned())
        }
    }

    /// `~basic_string()` — frees only when the long representation owns a buffer.
    #[inline]
    unsafe fn destroy(&mut self) {
        if self.is_long() {
            let w = self.rep.as_ptr() as *const usize;
            cxx_operator_delete(w.add(2).read_unaligned() as *mut u8);
        }
    }
}

/// `std::string s; s.resize(n);` — an empty string grown with NULs, which is exactly
/// what `append(n, '\0')` does, capacity rule included.
#[inline]
unsafe fn string_resized(out: *mut StdString, n: usize) {
    *out = StdString::EMPTY;
    if n != 0 {
        string_append(out, n, 0);
    }
}

// ---------------------------------------------------------------------------
// The unit's own helpers.
// ---------------------------------------------------------------------------

/// Bytes in a UTF-8 sequence given its leading byte. Transcribed verbatim; note the
/// 5- and 6-byte entries, which modern UTF-8 forbids but this table still reports, and
/// the two trailing `1`s for `0xFE`/`0xFF`.
#[rustfmt::skip]
const SEQUENCE_LENGTHS: [u8; 256] = [
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,
    2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,
    3,3,3,3,3,3,3,3,3,3,3,3,3,3,3,3,
    4,4,4,4,4,4,4,4,5,5,5,5,6,6,1,1,
];

/// `equal_sequence(const char*& begin, const char* end, const char* begin2)`.
///
/// Advances `begin` past one UTF-8 sequence if it matches. The continuation-byte scan
/// is `(b & 0xC0) != 0x80`, and it stops at `end` — so a truncated sequence at the end
/// of the haystack compares only the bytes that are there.
#[inline]
unsafe fn equal_sequence(begin: &mut *const c_char, end: *const c_char, begin2: *const c_char) -> bool {
    let b = *begin;
    if *b != *begin2 {
        return false;
    }
    let mut i = 1usize;
    if (*(b as *const u8)) & 0x80 != 0 {
        while b.add(i) != end {
            let ch = *(b.add(i) as *const u8);
            if (ch & (0x80 + 0x40)) != 0x80 {
                break;
            }
            if *b.add(i) != *begin2.add(i) {
                return false;
            }
            i += 1;
        }
    }
    *begin = b.add(i);
    true
}

#[inline]
unsafe fn sd_bytes(s: StringData) -> &'static [u8] {
    if s.size == 0 {
        &[]
    } else {
        core::slice::from_raw_parts(s.data as *const u8, s.size)
    }
}

/// Shared body of `equal_case_fold`.
unsafe fn equal_case_fold_impl(haystack: StringData, needle_upper: *const c_char, needle_lower: *const c_char) -> bool {
    // First a byte-wise pass: cheap, and rejects most non-matches.
    for i in 0..haystack.size {
        let c = *haystack.data.add(i);
        if *needle_lower.add(i) != c && *needle_upper.add(i) != c {
            return false;
        }
    }

    // Then the rigorous pass, one UTF-8 sequence at a time.
    let begin = haystack.data;
    let end = begin.add(haystack.size);
    let mut i = begin;
    while i != end {
        let off = i as usize - begin as usize;
        let mut a = i;
        let mut b = i;
        let ok_lower = equal_sequence(&mut a, end, needle_lower.add(off));
        // Note the short-circuit: the C++ evaluates equal_sequence(i, ...) on the SAME
        // `i` twice, and the first call only advances it when it returns true. Mirrored
        // by trying lower first and falling back to upper from the unadvanced position.
        if ok_lower {
            i = a;
            continue;
        }
        if equal_sequence(&mut b, end, needle_upper.add(off)) {
            i = b;
            continue;
        }
        return false;
    }
    true
}

// ===========================================================================
// The nine exported symbols.
// ===========================================================================

/// `realm::sequence_length(char)`
#[export_name = "_ZN5realm15sequence_lengthEc"]
pub extern "C" fn sequence_length(lead: c_char) -> usize {
    SEQUENCE_LENGTHS[lead as u8 as usize] as usize
}

/// `realm::utf8value(const char*)`
///
/// "No check for invalid utf8; may read out of bounds!" — the caller's contract, and
/// the port keeps it rather than adding a bounds check that would change behaviour.
#[export_name = "_ZN5realm9utf8valueEPKc"]
pub unsafe extern "C" fn utf8value(character: *const c_char) -> u32 {
    let c = character as *const u8;
    let len = SEQUENCE_LENGTHS[*c as usize] as usize;
    let mut res = *c as u32;
    if len == 1 {
        return res;
    }
    res &= (0x3fu32) >> (len - 1);
    for i in 1..len {
        res = (res << 6) | ((*c.add(i) as u32) & 0x3f);
    }
    res
}

/// `realm::case_map(StringData, bool)` → `util::Optional<std::string>`, via `sret`.
///
/// Byte-length preserving: only ASCII and the Latin-1 supplement are folded, because
/// those are the ranges where the mapped character encodes to the same number of bytes.
/// Everything else is validated and copied through. A malformed sequence yields `none`.
#[export_name = "_ZN5realm8case_mapENS_10StringDataEb"]
pub unsafe extern "C-unwind" fn case_map(out: *mut OptString, source: StringData, upper: bool) -> *mut OptString {
    let res = core::ptr::addr_of_mut!((*out).s);
    string_resized(res, source.size);
    (*out).engaged = false;

    let (rdata, _) = (*res).parts();
    let sz = source.size;
    let src = source.data;

    let mut i = 0usize;
    while i < sz {
        let mut c = *src.add(i);
        let int_val = c as u8 as u32; // char_traits<char>::to_int_type

        if int_val < 0x80 {
            if upper && (c as u8 >= b'a' && c as u8 <= b'z') {
                c = (c as u8 - 0x20) as c_char;
            } else if !upper && (c as u8 >= b'A' && c as u8 <= b'Z') {
                c = (c as u8 + 0x20) as c_char;
            }
        } else if (int_val & 0xE0) == 0xC0 {
            // 2-byte: the only multibyte range that is folded.
            if i + 2 > sz {
                return none(out);
            }
            c = *src.add(i + 1);
            if (c as u8) & 0xC0 != 0x80 {
                return none(out);
            }
            let u = (((int_val << 6) + ((c as u8 as u32) & 0x3F)) & 0x7FF) as u32;
            let u = if upper && (0xE0..=0xFE).contains(&u) && u != 0xF7 {
                u - 0x20
            } else if !upper && (0xC0..=0xDE).contains(&u) && u != 0xD7 {
                u + 0x20
            } else {
                u
            };
            *rdata.add(i) = ((u >> 6) | 0xC0) as u8 as c_char;
            i += 1;
            c = ((u & 0x3f) | 0x80) as u8 as c_char;
        } else if (int_val & 0xF0) == 0xE0 {
            if !copy_bytes(&mut i, &mut c, src, sz, rdata, 3) {
                return none(out);
            }
        } else if (int_val & 0xF8) == 0xF0 {
            if !copy_bytes(&mut i, &mut c, src, sz, rdata, 4) {
                return none(out);
            }
        } else {
            return none(out);
        }
        *rdata.add(i) = c;
        i += 1;
    }

    (*out).engaged = true;
    out
}

/// The `copy_bytes` lambda from `case_map`: copies `n-1` continuation bytes, validating
/// each, and leaves `c` holding the last one for the caller to store.
#[inline]
unsafe fn copy_bytes(
    i: &mut usize,
    c: &mut c_char,
    src: *const c_char,
    sz: usize,
    rdata: *mut c_char,
    n: usize,
) -> bool {
    if *i + n > sz {
        return false;
    }
    for _ in 1..n {
        *rdata.add(*i) = *c;
        *i += 1;
        *c = *src.add(*i);
        if (*c as u8) & 0xC0 != 0x80 {
            return false;
        }
    }
    true
}

/// Return `util::none`, freeing the partially built string first — the C++ `return {}`
/// destroys `result` on the way out.
#[inline]
unsafe fn none(out: *mut OptString) -> *mut OptString {
    (*out).s.destroy();
    (*out).s = StdString::EMPTY;
    (*out).engaged = false;
    out
}

/// `realm::case_map(StringData, bool, IgnoreErrorsTag)` → `std::string`, via `sret`.
///
/// `case_map(source, upper).value_or("")`. `IgnoreErrorsTag` is an empty class; the
/// Itanium ABI still gives it an argument slot, so it is declared as a byte.
#[export_name = "_ZN5realm8case_mapENS_10StringDataEbNS_15IgnoreErrorsTagE"]
pub unsafe extern "C-unwind" fn case_map_ignore_errors(
    out: *mut StdString,
    source: StringData,
    upper: bool,
    _tag: u8,
) -> *mut StdString {
    let mut opt = core::mem::MaybeUninit::<OptString>::uninit();
    let opt = opt.as_mut_ptr();
    case_map(opt, source, upper);
    if (*opt).engaged {
        // value_or on an rvalue moves: take the representation and leave the temporary
        // empty so its destructor frees nothing.
        core::ptr::copy_nonoverlapping(core::ptr::addr_of!((*opt).s), out, 1);
        (*opt).s = StdString::EMPTY;
    } else {
        *out = StdString::EMPTY;
    }
    (*opt).s.destroy();
    out
}

/// `realm::equal_case_fold(StringData, const char*, const char*)`
#[export_name = "_ZN5realm15equal_case_foldENS_10StringDataEPKcS2_"]
pub unsafe extern "C" fn equal_case_fold(
    haystack: StringData,
    needle_upper: *const c_char,
    needle_lower: *const c_char,
) -> bool {
    equal_case_fold_impl(haystack, needle_upper, needle_lower)
}

/// `realm::search_case_fold(StringData, const char*, const char*, size_t)`
///
/// Returns `haystack.size()` when not found — not `npos`.
#[export_name = "_ZN5realm16search_case_foldENS_10StringDataEPKcS2_m"]
pub unsafe extern "C" fn search_case_fold(
    haystack: StringData,
    needle_upper: *const c_char,
    needle_lower: *const c_char,
    needle_size: usize,
) -> usize {
    let mut i = 0usize;
    // The loop exits before `haystack.size() - i` can wrap, because it stops as soon as
    // the remaining length is below needle_size.
    while needle_size <= haystack.size - i {
        let sub = StringData {
            data: haystack.data.add(i),
            size: needle_size,
        };
        if equal_case_fold_impl(sub, needle_upper, needle_lower) {
            return i;
        }
        i += 1;
    }
    haystack.size
}

/// `realm::contains_ins(StringData, const char*, const char*, size_t, const std::array<uint8_t,256>&)`
///
/// Boyer-Moore with a skip table built by the caller. An empty needle is "contained"
/// only in a non-empty haystack, which is upstream's convention and not the usual one.
#[export_name = "_ZN5realm12contains_insENS_10StringDataEPKcS2_mRKNSt3__15arrayIhLm256EEE"]
pub unsafe extern "C" fn contains_ins(
    haystack: StringData,
    needle_upper: *const c_char,
    needle_lower: *const c_char,
    needle_size: usize,
    charmap: *const [u8; 256],
) -> bool {
    if needle_size == 0 {
        return haystack.size != 0;
    }

    let last_char_pos = needle_size - 1;
    let last_upper = *needle_upper.add(last_char_pos) as u8;
    let last_lower = *needle_lower.add(last_char_pos) as u8;

    let mut p = last_char_pos;
    while p < haystack.size {
        let c = *(haystack.data.add(p) as *const u8);

        if c == last_upper || c == last_lower {
            // `p - needle_size + 1` in C++ evaluates left to right on size_t: at the
            // first iteration p == needle_size - 1, so `p - needle_size` wraps to
            // SIZE_MAX and the `+ 1` wraps it back to 0. Benign there, but the
            // workspace sets overflow-checks = true, so a plain subtraction panics.
            // Mirrored as the same two wraps rather than rewritten as `p + 1 - n`.
            let start = p.wrapping_sub(needle_size).wrapping_add(1);
            let candidate = StringData {
                data: haystack.data.add(start),
                size: needle_size,
            };
            if equal_case_fold_impl(candidate, needle_upper, needle_lower) {
                return true;
            }
        }

        let skip = (*charmap)[c as usize];
        p += if skip == 0 { needle_size } else { skip as usize };
    }

    false
}

/// `realm::string_like_ins(StringData text, StringData upper, StringData lower)`
///
/// Forwards to `matchlike_ins(text, lower, upper)` — `lower` goes into the parameter
/// named `pattern_upper`. Preserved from the source; see the module comment for why a
/// negative control cannot detect it.
#[export_name = "_ZN5realm15string_like_insENS_10StringDataES0_S0_"]
pub unsafe extern "C" fn string_like_ins_3(text: StringData, upper: StringData, lower: StringData) -> bool {
    if text.data.is_null() || lower.data.is_null() {
        return text.data.is_null() && lower.data.is_null();
    }
    matchlike_ins(&text, &lower, &upper)
}

/// `realm::string_like_ins(StringData text, StringData pattern)`
#[export_name = "_ZN5realm15string_like_insENS_10StringDataES0_"]
pub unsafe extern "C-unwind" fn string_like_ins_2(text: StringData, pattern: StringData) -> bool {
    if text.data.is_null() || pattern.data.is_null() {
        return text.data.is_null() && pattern.data.is_null();
    }

    let mut upper = core::mem::MaybeUninit::<StdString>::uninit();
    let mut lower = core::mem::MaybeUninit::<StdString>::uninit();
    let upper = upper.as_mut_ptr();
    let lower = lower.as_mut_ptr();
    case_map_ignore_errors(upper, pattern, true, 0);
    case_map_ignore_errors(lower, pattern, false, 0);

    // c_str() then StringData(const char*), which is strlen-based -- an embedded NUL
    // truncates. Mirrored rather than using the string's own size.
    let (up, _) = (*upper).parts();
    let (lo, _) = (*lower).parts();
    let usd = StringData {
        data: up,
        size: c_strlen(up),
    };
    let lsd = StringData {
        data: lo,
        size: c_strlen(lo),
    };

    let r = matchlike_ins(&text, &lsd, &usd);

    (*upper).destroy();
    (*lower).destroy();
    r
}

#[inline]
unsafe fn c_strlen(p: *const c_char) -> usize {
    let mut n = 0usize;
    while *p.add(n) != 0 {
        n += 1;
    }
    n
}

#[allow(unused)]
fn _unused(_: *const c_void) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_length_table() {
        assert_eq!(sequence_length(b'A' as c_char), 1);
        assert_eq!(sequence_length(0x7Fu8 as i8 as c_char), 1);
        assert_eq!(sequence_length(0xC2u8 as i8 as c_char), 2);
        assert_eq!(sequence_length(0xE0u8 as i8 as c_char), 3);
        assert_eq!(sequence_length(0xF0u8 as i8 as c_char), 4);
        // The table still reports the obsolete 5- and 6-byte forms, and 1 for FE/FF.
        assert_eq!(sequence_length(0xF8u8 as i8 as c_char), 5);
        assert_eq!(sequence_length(0xFCu8 as i8 as c_char), 6);
        assert_eq!(sequence_length(0xFEu8 as i8 as c_char), 1);
        assert_eq!(sequence_length(0xFFu8 as i8 as c_char), 1);
    }

    #[test]
    fn utf8value_decodes() {
        unsafe {
            assert_eq!(utf8value(b"A\0".as_ptr() as *const c_char), 0x41);
            // U+00E9 = C3 A9
            assert_eq!(utf8value(b"\xC3\xA9\0".as_ptr() as *const c_char), 0xE9);
            // U+20AC = E2 82 AC
            assert_eq!(utf8value(b"\xE2\x82\xAC\0".as_ptr() as *const c_char), 0x20AC);
        }
    }
}
