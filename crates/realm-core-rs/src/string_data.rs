//! Port of `upstream/src/realm/string_data.cpp`.
//!
//! Five exported functions, all live in the linked binary:
//!
//! - `StringData::matchlike` / `matchlike_ins` — SQL `LIKE` matching with backtracking
//! - `murmur2_32`, `cityhash_64` — the two hash primitives, copied by realm from libc++
//! - `murmur2_or_cityhash` — selects between them on pointer width
//!
//! Unlike the two units ported before this one, the hashes can reach a `.realm`:
//! `index_string.cpp` uses `murmur2_or_cityhash` for string-index keys, and a hash that
//! disagrees with the C++ by a single bit puts different bytes in the index. Every
//! arithmetic decision below is therefore a format decision.
//!
//! ## Fidelity notes
//!
//! The C++ is a transcription of libc++'s `__murmur2_or_cityhash`, and it inherits
//! libc++'s reliance on wrapping unsigned arithmetic and on truncation at specific
//! widths. Mirrored rather than tidied, per `.claude/rules/format-fidelity.md`:
//!
//! - **`load4`/`load8` are `memcpy`, so they are native-endian and unaligned.**
//!   `from_ne_bytes` — *not* `from_le_bytes`. Realm only ships little-endian targets,
//!   so the two agree today; using `ne` keeps the Rust wrong in exactly the same way
//!   the C++ would be if that ever changed, which is the property we want.
//! - **Width of intermediate truncation matters.** In `hash_len_0_to_16`, `a << 3` is
//!   computed on a `uint_least32_t` and only *then* widened to 64 bits, so bits shifted
//!   past bit 31 are lost. Computing it in 64 bits would be "more correct" and would
//!   produce different hashes for inputs of 4..=8 bytes.
//! - **`rotate_by_at_least_1` is a right-rotate that is UB at shift 0** in C++
//!   (`val << 64`). `rotate()` exists solely to guard that case. Every call site passes
//!   a non-zero shift, so the guard never fires in practice; both are reproduced so the
//!   structure stays recognisable against the original.
//! - **`pattern.size() - 1` underflows for an empty pattern.** `wrapping_sub` keeps the
//!   `SIZE_MAX` the C++ computes; `len() - 1` would panic in debug instead.

use core::ptr;

/// `realm::StringData` — `upstream/src/realm/string_data.hpp:175`.
///
/// Plain 16-byte POD passed by const reference. `m_data` may be null when `m_size`
/// is zero, so it is never turned into a slice without checking.
#[repr(C)]
pub struct StringData {
    m_data: *const core::ffi::c_char,
    m_size: usize,
}

impl StringData {
    /// Borrow as bytes. Null + zero-length is representable and must not become a
    /// slice from a null pointer.
    ///
    /// # Safety
    /// `m_data` must point to `m_size` readable bytes, or `m_size` must be 0.
    unsafe fn as_bytes(&self) -> &[u8] {
        if self.m_size == 0 {
            &[]
        } else {
            core::slice::from_raw_parts(self.m_data as *const u8, self.m_size)
        }
    }
}

// ---------------------------------------------------------------------------
// matchlike
// ---------------------------------------------------------------------------

/// Mirror of the anonymous-namespace `matchlike<has_alternate_pattern>` template.
///
/// `alternate_pattern`, when present, differs from `pattern` only in case; the caller
/// guarantees equal lengths. The `goto no_match` in the C++ becomes a fallthrough into
/// the backtracking block at the bottom of the loop.
fn matchlike_impl(text: &[u8], pattern: &[u8], alternate_pattern: Option<&[u8]>) -> bool {
    debug_assert!(alternate_pattern.map_or(true, |a| a.len() == pattern.len()));

    let mut textpos: Vec<usize> = Vec::new();
    let mut patternpos: Vec<usize> = Vec::new();
    let mut p1 = 0usize; // position in text (haystack)
    let mut p2 = 0usize; // position in pattern (needle)

    loop {
        // Each arm either returns, `continue`s, or falls through to the `no_match`
        // block below — matching the control flow of the C++ `goto`.
        if p1 == text.len() {
            // At the end of the text: a match if also at the end of the pattern, or if
            // the one remaining pattern character is a multi-character wildcard.
            if p2 == pattern.len() {
                return true;
            }
            // `pattern.len() - 1` underflows to SIZE_MAX for an empty pattern in the
            // C++; p2 is 0 there, so the comparison is simply false. Mirrored.
            if p2 == pattern.len().wrapping_sub(1) && pattern[p2] == b'*' {
                return true;
            }
            // fall through to no_match
        } else if p2 == pattern.len() {
            // Hit the end of the pattern without consuming all the text.
            // fall through to no_match
        } else if pattern[p2] == b'*' {
            // Multi-character wildcard: remember the position in case we backtrack.
            textpos.push(p1);
            p2 += 1;
            patternpos.push(p2);
            continue;
        } else if pattern[p2] == b'?' {
            // `?` matches one UTF-8 character, which may span several bytes.
            if (text[p1] & 0x80) == 0 {
                p1 += 1;
                p2 += 1;
            } else {
                let mut p = 1usize;
                while p1 + p != text.len() && (text[p1 + p] & 0xc0) == 0x80 {
                    p += 1;
                }
                p1 += p;
                p2 += 1;
            }
            continue;
        } else if pattern[p2] == text[p1] {
            p1 += 1;
            p2 += 1;
            continue;
        } else if alternate_pattern.is_some_and(|alt| alt[p2] == text[p1]) {
            p1 += 1;
            p2 += 1;
            continue;
        }

        // no_match:
        if textpos.is_empty() {
            // Outermost level of matching, so the text did not match.
            return false;
        }

        if p1 == text.len() {
            // Hit the end of the text without a match, so backtrack.
            textpos.pop();
            patternpos.pop();
        }

        if textpos.is_empty() {
            // Last backtrack attempt exhausted.
            return false;
        }

        // Reattempt the match from the next character: `p1 = ++textpos.back()`.
        let last = textpos.last_mut().expect("checked non-empty above");
        *last += 1;
        p1 = *last;
        p2 = *patternpos.last().expect("stacks are pushed and popped together");
    }
}

/// `realm::StringData::matchlike(StringData const&, StringData const&)`
///
/// # Safety
/// Both references must be valid `StringData` objects, as guaranteed by the C++ caller.
#[export_name = "_ZN5realm10StringData9matchlikeERKS0_S2_"]
pub unsafe extern "C" fn matchlike(text: &StringData, pattern: &StringData) -> bool {
    matchlike_impl(text.as_bytes(), pattern.as_bytes(), None)
}

/// `realm::StringData::matchlike_ins(StringData const&, StringData const&, StringData const&)`
///
/// # Safety
/// All three references must be valid `StringData` objects, as guaranteed by the
/// C++ caller. `pattern_upper` and `pattern_lower` must have equal length.
#[export_name = "_ZN5realm10StringData13matchlike_insERKS0_S2_S2_"]
pub unsafe extern "C" fn matchlike_ins(
    text: &StringData,
    pattern_upper: &StringData,
    pattern_lower: &StringData,
) -> bool {
    matchlike_impl(
        text.as_bytes(),
        pattern_upper.as_bytes(),
        Some(pattern_lower.as_bytes()),
    )
}

// ---------------------------------------------------------------------------
// hashes
// ---------------------------------------------------------------------------

/// `memcpy` of 4 bytes into a `uint_least32_t`: native-endian, alignment-agnostic.
///
/// # Safety
/// `data` must point to at least 4 readable bytes.
#[inline]
unsafe fn load4(data: *const u8) -> u32 {
    let mut buf = [0u8; 4];
    ptr::copy_nonoverlapping(data, buf.as_mut_ptr(), 4);
    u32::from_ne_bytes(buf)
}

/// `memcpy` of 8 bytes into a `uint_least64_t`: native-endian, alignment-agnostic.
///
/// # Safety
/// `data` must point to at least 8 readable bytes.
#[inline]
unsafe fn load8(data: *const u8) -> u64 {
    let mut buf = [0u8; 8];
    ptr::copy_nonoverlapping(data, buf.as_mut_ptr(), 8);
    u64::from_ne_bytes(buf)
}

/// `realm::murmur2_32(unsigned char const*, size_t)`
///
/// # Safety
/// `data` must point to at least `len` readable bytes.
#[export_name = "_ZN5realm10murmur2_32EPKhm"]
pub unsafe extern "C" fn murmur2_32(data: *const u8, len: usize) -> u32 {
    const M: u32 = 0x5bd1_e995;
    const R: u32 = 24;

    let mut data = data;
    let mut len = len;
    // `h` seeds from the length truncated to 32 bits.
    let mut h: u32 = len as u32;

    while len >= 4 {
        let mut k = load4(data);
        k = k.wrapping_mul(M);
        k ^= k >> R;
        k = k.wrapping_mul(M);
        h = h.wrapping_mul(M);
        h ^= k;
        data = data.add(4);
        len -= 4;
    }

    // Tail. The C++ falls through 3 -> 2 -> 1, and only the len==1 arm multiplies.
    if len == 3 {
        h ^= u32::from(*data.add(2)) << 16;
    }
    if len >= 2 {
        h ^= u32::from(*data.add(1)) << 8;
    }
    if len >= 1 {
        h ^= u32::from(*data);
        h = h.wrapping_mul(M);
    }

    h ^= h >> 13;
    h = h.wrapping_mul(M);
    h ^= h >> 15;
    h
}

const K0: u64 = 0xc3a5_c85c_97cb_3127;
const K1: u64 = 0xb492_b66f_be98_f273;
const K2: u64 = 0x9ae1_6a3b_2f90_404f;
const K3: u64 = 0xc949_d7c7_509e_6557;

/// `(val >> shift) | (val << (64 - shift))` — a right-rotate.
///
/// UB in the C++ when `shift == 0` (`val << 64`); every call site passes a non-zero
/// shift. `debug_assert` documents the precondition without changing release codegen.
#[inline]
fn rotate_by_at_least_1(val: u64, shift: i32) -> u64 {
    debug_assert!(shift > 0 && shift < 64);
    (val >> shift) | (val << (64 - shift))
}

/// The C++ `rotate()`, whose only job is to make `shift == 0` well-defined.
#[inline]
fn rotate(val: u64, shift: i32) -> u64 {
    if shift == 0 {
        val
    } else {
        rotate_by_at_least_1(val, shift)
    }
}

#[inline]
fn shift_mix(val: u64) -> u64 {
    val ^ (val >> 47)
}

#[inline]
fn hash_len_16(u: u64, v: u64) -> u64 {
    const MUL: u64 = 0x9ddf_ea08_eb38_2d69;
    let mut a = (u ^ v).wrapping_mul(MUL);
    a ^= a >> 47;
    let mut b = (v ^ a).wrapping_mul(MUL);
    b ^= b >> 47;
    b = b.wrapping_mul(MUL);
    b
}

/// # Safety
/// `data` must point to at least `len` readable bytes, `len <= 16`.
unsafe fn hash_len_0_to_16(data: *const u8, len: usize) -> u64 {
    if len > 8 {
        let a = load8(data);
        let b = load8(data.add(len - 8));
        return hash_len_16(a, rotate_by_at_least_1(b.wrapping_add(len as u64), len as i32)) ^ b;
    }
    if len >= 4 {
        let a = load4(data);
        let b = load4(data.add(len - 4));
        // `a << 3` is 32-bit arithmetic in the C++ and only then widened, so bits
        // above bit 31 are discarded. Doing this in u64 would change the hash.
        let a_shifted = a << 3;
        return hash_len_16((len as u64).wrapping_add(u64::from(a_shifted)), u64::from(b));
    }
    if len > 0 {
        let a = *data;
        let b = *data.add(len >> 1);
        let c = *data.add(len - 1);
        let y = u32::from(a).wrapping_add(u32::from(b) << 8);
        let z = (len as u32).wrapping_add(u32::from(c) << 2);
        return shift_mix(u64::from(y).wrapping_mul(K2) ^ u64::from(z).wrapping_mul(K3))
            .wrapping_mul(K2);
    }
    K2
}

/// # Safety
/// `data` must point to at least `len` readable bytes, `17 <= len <= 32`.
unsafe fn hash_len_17_to_32(data: *const u8, len: usize) -> u64 {
    let a = load8(data).wrapping_mul(K1);
    let b = load8(data.add(8));
    let c = load8(data.add(len - 8)).wrapping_mul(K2);
    let d = load8(data.add(len - 16)).wrapping_mul(K0);
    hash_len_16(
        rotate(a.wrapping_sub(b), 43)
            .wrapping_add(rotate(c, 30))
            .wrapping_add(d),
        a.wrapping_add(rotate(b ^ K3, 20))
            .wrapping_sub(c)
            .wrapping_add(len as u64),
    )
}

/// # Safety
/// `data` must point to at least `len` readable bytes, `33 <= len <= 64`.
unsafe fn hash_len_33_to_64(data: *const u8, len: usize) -> u64 {
    let mut z = load8(data.add(24));
    let mut a = load8(data).wrapping_add((len as u64).wrapping_add(load8(data.add(len - 16))).wrapping_mul(K0));
    let mut b = rotate(a.wrapping_add(z), 52);
    let mut c = rotate(a, 37);
    a = a.wrapping_add(load8(data.add(8)));
    c = c.wrapping_add(rotate(a, 7));
    a = a.wrapping_add(load8(data.add(16)));
    let vf = a.wrapping_add(z);
    let vs = b.wrapping_add(rotate(a, 31)).wrapping_add(c);
    a = load8(data.add(16)).wrapping_add(load8(data.add(len - 32)));
    z = z.wrapping_add(load8(data.add(len - 8)));
    b = rotate(a.wrapping_add(z), 52);
    c = rotate(a, 37);
    a = a.wrapping_add(load8(data.add(len - 24)));
    c = c.wrapping_add(rotate(a, 7));
    a = a.wrapping_add(load8(data.add(len - 16)));
    let wf = a.wrapping_add(z);
    let ws = b.wrapping_add(rotate(a, 31)).wrapping_add(c);
    let r = shift_mix(
        vf.wrapping_add(ws)
            .wrapping_mul(K2)
            .wrapping_add(wf.wrapping_add(vs).wrapping_mul(K0)),
    );
    shift_mix(r.wrapping_mul(K0).wrapping_add(vs)).wrapping_mul(K2)
}

#[inline]
fn weak_hash_len_32_with_seeds_vals(w: u64, x: u64, y: u64, z: u64, a: u64, b: u64) -> (u64, u64) {
    let mut a = a;
    let mut b = b;
    a = a.wrapping_add(w);
    b = rotate(b.wrapping_add(a).wrapping_add(z), 21);
    let c = a;
    a = a.wrapping_add(x);
    a = a.wrapping_add(y);
    b = b.wrapping_add(rotate(a, 44));
    (a.wrapping_add(z), b.wrapping_add(c))
}

/// # Safety
/// `data` must point to at least 32 readable bytes.
#[inline]
unsafe fn weak_hash_len_32_with_seeds(data: *const u8, a: u64, b: u64) -> (u64, u64) {
    weak_hash_len_32_with_seeds_vals(
        load8(data),
        load8(data.add(8)),
        load8(data.add(16)),
        load8(data.add(24)),
        a,
        b,
    )
}

/// `realm::cityhash_64(unsigned char const*, size_t)`
///
/// # Safety
/// `data` must point to at least `len` readable bytes.
#[export_name = "_ZN5realm11cityhash_64EPKhm"]
pub unsafe extern "C" fn cityhash_64(data: *const u8, len: usize) -> u64 {
    if len <= 32 {
        if len <= 16 {
            return hash_len_0_to_16(data, len);
        }
        return hash_len_17_to_32(data, len);
    } else if len <= 64 {
        return hash_len_33_to_64(data, len);
    }

    let mut data = data;
    let mut len = len;

    let mut x = load8(data.add(len - 40));
    let mut y = load8(data.add(len - 16)).wrapping_add(load8(data.add(len - 56)));
    let mut z = hash_len_16(
        load8(data.add(len - 48)).wrapping_add(len as u64),
        load8(data.add(len - 24)),
    );
    let mut v = weak_hash_len_32_with_seeds(data.add(len - 64), len as u64, z);
    let mut w = weak_hash_len_32_with_seeds(data.add(len - 32), y.wrapping_add(K1), x);
    x = x.wrapping_mul(K1).wrapping_add(load8(data));

    // Decrease len to the nearest multiple of 64, and operate on 64-byte chunks.
    len = (len - 1) & !63usize;
    loop {
        x = rotate(
            x.wrapping_add(y)
                .wrapping_add(v.0)
                .wrapping_add(load8(data.add(8))),
            37,
        )
        .wrapping_mul(K1);
        y = rotate(
            y.wrapping_add(v.1).wrapping_add(load8(data.add(48))),
            42,
        )
        .wrapping_mul(K1);
        x ^= w.1;
        y = y.wrapping_add(v.0).wrapping_add(load8(data.add(40)));
        z = rotate(z.wrapping_add(w.0), 33).wrapping_mul(K1);
        v = weak_hash_len_32_with_seeds(data, v.1.wrapping_mul(K1), x.wrapping_add(w.0));
        w = weak_hash_len_32_with_seeds(
            data.add(32),
            z.wrapping_add(w.1),
            y.wrapping_add(load8(data.add(16))),
        );
        core::mem::swap(&mut z, &mut x);
        data = data.add(64);
        len -= 64;
        if len == 0 {
            break;
        }
    }

    hash_len_16(
        hash_len_16(v.0, w.0)
            .wrapping_add(shift_mix(y).wrapping_mul(K1))
            .wrapping_add(z),
        hash_len_16(v.1, w.1).wrapping_add(x),
    )
}

/// `realm::murmur2_or_cityhash(unsigned char const*, size_t)`
///
/// The C++ selects via `Murmur2OrCityHash<sizeof(void*)>`: cityhash on 64-bit
/// pointers, murmur2 on 32-bit. Mirrored with `target_pointer_width` so the choice
/// tracks the same property rather than being hardcoded to the target we happen to
/// build today.
///
/// # Safety
/// `data` must point to at least `len` readable bytes.
#[export_name = "_ZN5realm19murmur2_or_cityhashEPKhm"]
pub unsafe extern "C" fn murmur2_or_cityhash(data: *const u8, len: usize) -> usize {
    #[cfg(target_pointer_width = "64")]
    {
        cityhash_64(data, len) as usize
    }
    #[cfg(target_pointer_width = "32")]
    {
        murmur2_32(data, len) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sd(s: &str) -> StringData {
        StringData {
            m_data: s.as_ptr() as *const core::ffi::c_char,
            m_size: s.len(),
        }
    }

    fn like(text: &str, pattern: &str) -> bool {
        matchlike_impl(text.as_bytes(), pattern.as_bytes(), None)
    }

    #[test]
    fn matchlike_literals_and_anchors() {
        assert!(like("", ""));
        assert!(!like("a", ""));
        assert!(!like("", "a"));
        assert!(like("abc", "abc"));
        assert!(!like("abc", "abd"));
    }

    #[test]
    fn empty_pattern_does_not_underflow() {
        // `pattern.size() - 1` is SIZE_MAX in the C++ for an empty pattern; the guard
        // is that p2 == 0 never equals it. Exercises the wrapping_sub mirror.
        assert!(like("", ""));
        assert!(!like("x", ""));
    }

    #[test]
    fn star_wildcard_backtracks() {
        assert!(like("", "*"));
        assert!(like("abc", "*"));
        assert!(like("abc", "a*"));
        assert!(like("abc", "*c"));
        assert!(like("abc", "a*c"));
        assert!(like("abcabc", "*abc"));
        assert!(!like("abc", "*d"));
        // Backtracking: the first `*` must give back characters for the tail to match.
        assert!(like("aaa", "*a"));
        assert!(like("xayaz", "*a*a*"));
        assert!(!like("xayaz", "*a*a*a*"));
    }

    #[test]
    fn question_mark_consumes_one_utf8_character() {
        assert!(like("a", "?"));
        assert!(!like("ab", "?"));
        // 'é' is two bytes; '?' must consume both.
        assert!(like("é", "?"));
        assert!(like("aéb", "a?b"));
        // A 4-byte emoji is still one character.
        assert!(like("😀", "?"));
        assert!(!like("😀", "??"));
    }

    #[test]
    fn matchlike_ins_accepts_either_case_pattern() {
        let text = sd("Hello");
        let upper = sd("HELLO");
        let lower = sd("hello");
        // SAFETY: all three StringData values borrow live local strings.
        assert!(unsafe { matchlike_ins(&text, &upper, &lower) });

        let text2 = sd("HeLLo");
        assert!(unsafe { matchlike_ins(&text2, &upper, &lower) });

        let text3 = sd("Hellx");
        assert!(!unsafe { matchlike_ins(&text3, &upper, &lower) });
    }

    #[test]
    fn matchlike_handles_null_data_for_empty_strings() {
        let empty = StringData {
            m_data: core::ptr::null(),
            m_size: 0,
        };
        // SAFETY: size 0 means as_bytes() never dereferences the null pointer.
        assert!(unsafe { matchlike(&empty, &empty) });
    }

    /// The hashes are *not* checked against reference values here. Hardcoding vectors
    /// would only prove this implementation agrees with itself. The real check is
    /// `migration/checks/run_string_data_differential.sh`, which builds one driver
    /// twice — against the C++ object and against the Rust staticlib — and requires
    /// byte-identical dumps. See `migration/JOURNAL.md`.
    ///
    /// What is worth asserting in-crate is the structural invariant the differential
    /// cannot express: the length-branch boundaries are hit at all, so a driver corpus
    /// that missed one would be visible as a gap here rather than as silent coverage.
    #[test]
    fn hash_length_branches_are_all_reachable() {
        let buf: Vec<u8> = (0u8..=255).cycle().take(200).collect();
        // 0/1-3/4-8/9-16 -> hash_len_0_to_16, 17-32, 33-64, >64 main loop.
        for len in [0usize, 1, 3, 4, 8, 9, 16, 17, 32, 33, 64, 65, 128, 129, 200] {
            // SAFETY: buf has 200 bytes; every len above is <= 200.
            let c = unsafe { cityhash_64(buf.as_ptr(), len) };
            let m = unsafe { murmur2_32(buf.as_ptr(), len) };
            // No reference value to compare against — the assertion is that these
            // terminate and are deterministic, which the differential then pins down.
            // SAFETY: as above.
            assert_eq!(c, unsafe { cityhash_64(buf.as_ptr(), len) });
            assert_eq!(m, unsafe { murmur2_32(buf.as_ptr(), len) });
        }
        // The empty case is fixed by the algorithm and safe to state outright.
        // SAFETY: reading 0 bytes from a valid pointer.
        assert_eq!(unsafe { cityhash_64(buf.as_ptr(), 0) }, K2);
    }

    #[test]
    fn murmur2_or_cityhash_selects_by_pointer_width() {
        let b = b"the quick brown fox jumps over the lazy dog";
        // SAFETY: b is a live byte slice.
        let combined = unsafe { murmur2_or_cityhash(b.as_ptr(), b.len()) };
        #[cfg(target_pointer_width = "64")]
        // SAFETY: as above.
        assert_eq!(combined as u64, unsafe { cityhash_64(b.as_ptr(), b.len()) });
        #[cfg(target_pointer_width = "32")]
        // SAFETY: as above.
        assert_eq!(combined as u32, unsafe { murmur2_32(b.as_ptr(), b.len()) });
    }
}
