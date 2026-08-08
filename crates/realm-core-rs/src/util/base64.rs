//! Port of `upstream/src/realm/util/base64.cpp`.
//!
//! # Does this unit reach the file?
//!
//! No. Every call site — `to_json.cpp`, `uuid.cpp` (`UUID::to_base64`),
//! `util/serializer.cpp`, `util/bson/bson.cpp`, and the (disabled) sync/app code —
//! consumes the result as *text*: JSON output, query serialisation, JWT payloads.
//! Nothing here decides an element width, an alignment, a ref encoding, or an
//! allocation order, and no byte produced by this file is written into a `.realm`.
//!
//! That matters for how this unit is judged: `make diff-test` cannot distinguish a
//! correct port of base64 from a wrong one, because it never exercises it. The gate
//! still has to pass — it proves the *link* is intact and nothing else regressed —
//! but the evidence that this port is correct comes from
//! `migration/checks/run_base64_differential.sh`, which builds one driver twice —
//! against the oracle's C++ object and against this crate's staticlib — and requires
//! the two dumps to be byte-identical.
//!
//! # The ABI is the hard part
//!
//! These are C++ functions with C++ types in their signatures, so the Rust has to
//! reproduce the Itanium ABI by hand. Verified against
//! `build/oracle/.../util/base64.cpp.o` on this checkout (x86_64, libc++):
//!
//! | C++ type | layout | how it is passed |
//! |---|---|---|
//! | `Span<T, dynamic_extent>` | `{T* data; size_t size}`, 16 B | 2 GPRs (trivially copyable) |
//! | `std::optional<size_t>` | `{size_t value; bool engaged}` @0,@8, 16 B | returned in `rax`:`dl` |
//! | `std::vector<char>` | `{char* begin; char* end; char* cap}`, 24 B | — |
//! | `std::optional<std::vector<char>>` | vector @0, `bool` @24, 32 B | **sret** (`rdi`), non-trivial dtor |
//!
//! The `optional<size_t>` register return was read straight off the disassembly:
//! the `none` path is `xorl %eax,%eax; xorl %edx,%edx`, the engaged path ends
//! `movb $0x1,%dl` with the value already in `rax`.
//!
//! # Ownership across the boundary
//!
//! `base64_decode_to_vector` hands a `std::vector<char>` back to C++, which will
//! destroy it with `operator delete`. So the buffer must come from `operator new`
//! (`_Znwm`), not from Rust's allocator — mixing them is a heap corruption that no
//! test in this repo would catch. `base64.cpp.o` imports the *unsized*
//! `operator delete(void*)` (`_ZdlPv`), so that is what the failure path calls.
//!
//! `operator new` throws `std::bad_alloc`, and the C++ `base64_decode_to_vector` is
//! not `noexcept`, so that exception is part of its contract. The Rust function is
//! declared `extern "C-unwind"` to let it propagate; the frame holds only raw
//! pointers, so there is nothing to drop on the way out.
//!
//! # Assertions
//!
//! Every `REALM_ASSERT`/`REALM_ASSERT_EX` in the C++ compiles to
//! `static_cast<void>(sizeof bool(cond))` in this build — `REALM_ENABLE_ASSERTIONS`
//! is off and `REALM_DEBUG` is undefined in Release, and the object file confirms it
//! (no undefined `realm::util::terminate`). They are mirrored as `debug_assert!`,
//! which is likewise absent from the `--release` staticlib the hybrid links.
//!
//! Consequently the release-mode arithmetic can overflow exactly as the C++ does, so
//! the size computations use `wrapping_*` rather than Rust's checked defaults. That
//! is a deliberate mirror, not an oversight: see `.claude/rules/format-fidelity.md`.

use core::ffi::c_char;

/// `realm::util::Span<const char, dynamic_extent>`.
#[repr(C)]
pub struct SpanConstChar {
    data: *const c_char,
    size: usize,
}

/// `realm::util::Span<char, dynamic_extent>`.
#[repr(C)]
pub struct SpanChar {
    data: *mut c_char,
    size: usize,
}

/// `std::optional<size_t>` — value at offset 0, engaged flag at offset 8.
#[repr(C)]
pub struct OptSize {
    value: usize,
    engaged: bool,
}

impl OptSize {
    const NONE: Self = OptSize {
        value: 0,
        engaged: false,
    };
    fn some(value: usize) -> Self {
        OptSize {
            value,
            engaged: true,
        }
    }
}

/// `std::vector<char>` under libc++: three pointers, no small-buffer optimisation.
#[repr(C)]
pub struct VectorChar {
    begin: *mut c_char,
    end: *mut c_char,
    cap: *mut c_char,
}

/// `std::optional<std::vector<char>>`.
#[repr(C)]
pub struct OptVectorChar {
    vec: VectorChar,
    engaged: bool,
}

const _: () = {
    assert!(core::mem::size_of::<SpanConstChar>() == 16);
    assert!(core::mem::size_of::<SpanChar>() == 16);
    assert!(core::mem::size_of::<OptSize>() == 16);
    assert!(core::mem::size_of::<VectorChar>() == 24);
    // 24 bytes of vector + 1 bool + 7 padding. If this ever fails the sret layout is
    // wrong and every `base64_decode_to_vector` result is garbage in a way that only
    // shows up as a crash somewhere else entirely.
    assert!(core::mem::size_of::<OptVectorChar>() == 32);
};

extern "C-unwind" {
    /// `operator new(unsigned long)`. Throws `std::bad_alloc`.
    #[link_name = "_Znwm"]
    fn cxx_operator_new(size: usize) -> *mut u8;
    /// `operator delete(void*)` — the unsized form, matching what `base64.cpp.o` imports.
    #[link_name = "_ZdlPv"]
    fn cxx_operator_delete(ptr: *mut u8);
}

// Transcribed verbatim from g_base64_encoding_chars.
const ENCODING_CHARS: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Byte classes used by the decode table, matching the C++ `b64_byte_type` enum.
const EQUALS: u32 = 64;
const WHITESPACE: u32 = 65;
const INVALID: u32 = 66;

/// Transcribed verbatim from `g_base64_chars`, 16 entries per row.
///
/// Two quirks worth not "cleaning up", both of which change which inputs are
/// accepted: carriage return (0x0D) is *invalid*, not whitespace — only tab (0x09),
/// line feed (0x0A) and space (0x20) are skipped — and the table accepts the
/// URL-safe alphabet's `-` (0x2D) and `_` (0x5F) as 62 and 63 alongside `+` and `/`.
#[rustfmt::skip]
const DECODE_TABLE: [u8; 256] = [
    66, 66, 66, 66, 66, 66, 66, 66, 66, 65, 65, 66, 66, 66, 66, 66,
    66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66,
    65, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 62, 66, 62, 66, 63,
    52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 66, 66, 66, 64, 66, 66,
    66,  0,  1,  2,  3,  4,  5,  6,  7,  8,  9, 10, 11, 12, 13, 14,
    15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 66, 66, 66, 66, 63,
    66, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40,
    41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 66, 66, 66, 66, 66,
    66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66,
    66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66,
    66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66,
    66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66,
    66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66,
    66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66,
    66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66,
    66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66, 66,
];

/// Mirror of `base64_encoded_size()` (inline in the header, so it is not a symbol we
/// have to export — but the .cpp recomputes it and we must agree with it).
///
/// `wrapping_*`: the C++ guards this with two `REALM_ASSERT_EX`s that are compiled
/// out here, leaving plain unsigned wraparound.
#[inline]
fn encoded_size(n: usize) -> usize {
    4usize.wrapping_mul(n.wrapping_add(2) / 3)
}

/// Mirror of `base64_decoded_size()`.
#[inline]
fn decoded_size(n: usize) -> usize {
    n.wrapping_mul(3).wrapping_add(3) / 4
}

// ---------------------------------------------------------------------------
// realm::util::base64_encode(Span<const char>, Span<char>) noexcept
// ---------------------------------------------------------------------------
#[export_name = "_ZN5realm4util13base64_encodeENS0_4SpanIKcLm18446744073709551615EEENS1_IcLm18446744073709551615EEE"]
pub unsafe extern "C" fn base64_encode(in_buffer: SpanConstChar, out_buffer: SpanChar) -> usize {
    debug_assert!(in_buffer.size < usize::MAX - 2);
    debug_assert!(in_buffer.size < 3 * (usize::MAX / 4) - 2);
    let encoded = encoded_size(in_buffer.size);
    debug_assert!(out_buffer.size >= encoded);

    let input = in_buffer.data.cast::<u8>();
    let output = out_buffer.data.cast::<u8>();

    // Mirrors the C++ loop shape exactly, including the redundant `i < size` test on
    // octet_a (always true on entry) and the zero-fill of the missing octets.
    let mut i = 0usize;
    let mut j = 0usize;
    while i < in_buffer.size {
        let octet_a = {
            let v = *input.add(i) as u32;
            i += 1;
            v
        };
        let octet_b = if i < in_buffer.size {
            let v = *input.add(i) as u32;
            i += 1;
            v
        } else {
            0
        };
        let octet_c = if i < in_buffer.size {
            let v = *input.add(i) as u32;
            i += 1;
            v
        } else {
            0
        };

        let triple = (octet_a << 0x10) + (octet_b << 0x08) + octet_c;

        *output.add(j) = ENCODING_CHARS[((triple >> (3 * 6)) & 0x3F) as usize];
        *output.add(j + 1) = ENCODING_CHARS[((triple >> (2 * 6)) & 0x3F) as usize];
        *output.add(j + 2) = ENCODING_CHARS[((triple >> 6) & 0x3F) as usize];
        *output.add(j + 3) = ENCODING_CHARS[(triple & 0x3F) as usize];
        j += 4;
    }

    // The last zero, one or two characters must be set to '='.
    match in_buffer.size % 3 {
        1 => {
            *output.add(encoded - 1) = b'=';
            *output.add(encoded - 2) = b'=';
        }
        2 => {
            *output.add(encoded - 1) = b'=';
        }
        _ => {}
    }

    encoded
}

// ---------------------------------------------------------------------------
// realm::util::base64_decode(Span<const char>, Span<char>) noexcept
// ---------------------------------------------------------------------------
#[export_name = "_ZN5realm4util13base64_decodeENS0_4SpanIKcLm18446744073709551615EEENS1_IcLm18446744073709551615EEE"]
pub unsafe extern "C" fn base64_decode(input: SpanConstChar, out_buffer: SpanChar) -> OptSize {
    debug_assert!(input.size < usize::MAX / 3);
    debug_assert!(out_buffer.size >= decoded_size(input.size));

    let p = input.data.cast::<u8>();
    let o = out_buffer.data.cast::<u8>();
    let mut written_at = 0usize; // stands in for the C++ `char* restrict o` cursor

    let mut bytes_written = 0usize;
    let mut num_trailing_equals = 0usize;
    let mut buffer: u32 = 0;
    let mut buffer_size = 0usize;

    let mut i = 0usize;
    while i < input.size {
        let x = DECODE_TABLE[*p.add(i) as usize] as u32;
        i += 1;

        match x {
            EQUALS => {
                num_trailing_equals += 1;
                continue;
            }
            WHITESPACE => continue,
            INVALID => return OptSize::NONE,
            _ => {}
        }

        if num_trailing_equals > 0 {
            return OptSize::NONE; // data after the end-padding
        }

        debug_assert!(x < 64);
        // `unsigned int` in the C++. buffer_size is < 4 here so at most 18 bits are
        // live before the shift; this cannot overflow in either language.
        buffer = buffer << 6 | x;
        buffer_size += 1;

        if buffer_size == 4 {
            *o.add(written_at) = ((buffer >> 16) & 0xff) as u8;
            *o.add(written_at + 1) = ((buffer >> 8) & 0xff) as u8;
            *o.add(written_at + 2) = (buffer & 0xff) as u8;
            written_at += 3;
            buffer = 0;
            buffer_size = 0;
            bytes_written += 3;
        }
    }

    // no padding
    //
    // Note this counts *all* input characters including whitespace and '=', not just
    // the valid ones — so unpadded input containing whitespace is classified by its
    // raw length. Mirrored as-is.
    let extra = input.size % 4;
    if num_trailing_equals == 0 && extra > 1 {
        num_trailing_equals = 4 - extra;
    }

    // trailing bytes
    if num_trailing_equals == 0 {
        if buffer_size != 0 {
            return OptSize::NONE; // input was not sufficiently padded
        }
    } else if num_trailing_equals == 1 {
        *o.add(written_at) = ((buffer >> 10) & 0xff) as u8;
        *o.add(written_at + 1) = ((buffer >> 2) & 0xff) as u8;
        bytes_written += 2;
    } else if num_trailing_equals == 2 {
        *o.add(written_at) = ((buffer >> 4) & 0xff) as u8;
        bytes_written += 1;
    } else {
        return OptSize::NONE;
    }

    OptSize::some(bytes_written)
}

// ---------------------------------------------------------------------------
// realm::util::base64_decode_to_vector(Span<const char>)
// ---------------------------------------------------------------------------
#[export_name = "_ZN5realm4util23base64_decode_to_vectorENS0_4SpanIKcLm18446744073709551615EEE"]
pub unsafe extern "C-unwind" fn base64_decode_to_vector(encoded: SpanConstChar) -> OptVectorChar {
    let max_size = decoded_size(encoded.size);

    // `std::vector<char> decoded(max_size)`:
    //   - max_size == 0 allocates nothing and leaves all three pointers null. That is
    //     not cosmetic — returning a heap pointer here would make an empty result
    //     compare unequal to the oracle's and would hand C++ a block it frees with a
    //     capacity it never asked for.
    //   - otherwise it value-initialises, i.e. zero-fills.
    //
    // DECODE_OVERRUN_SLACK is the one place this port deliberately allocates
    // differently from the C++; see the constant's comment for the proof that it is
    // unobservable and why the alternative is a heap overflow.
    let begin: *mut u8 = if max_size == 0 {
        core::ptr::null_mut()
    } else {
        let p = cxx_operator_new(max_size + DECODE_OVERRUN_SLACK);
        core::ptr::write_bytes(p, 0, max_size + DECODE_OVERRUN_SLACK);
        p
    };

    let actual = base64_decode(
        SpanConstChar {
            data: encoded.data,
            size: encoded.size,
        },
        SpanChar {
            data: begin.cast::<c_char>(),
            size: max_size,
        },
    );

    if !actual.engaged {
        // `decoded` goes out of scope on the C++ `return std::nullopt` path.
        if !begin.is_null() {
            cxx_operator_delete(begin);
        }
        return OptVectorChar {
            vec: VectorChar {
                begin: core::ptr::null_mut(),
                end: core::ptr::null_mut(),
                cap: core::ptr::null_mut(),
            },
            engaged: false,
        };
    }

    // `decoded.resize(*actual_size)`. The obvious reading is that this only ever
    // shrinks — and that reading is wrong, which the differential check caught:
    // `base64_decode` can report one byte MORE than `base64_decoded_size` allowed for
    // (see DECODE_OVERRUN_SLACK), and then `resize` grows and reallocates. Getting
    // this wrong yields a vector whose end pointer is past its capacity: valid-looking
    // until something reads `capacity()` or the allocator notices.
    let size = max_size; // the vector's size() at this point
    let new_size = actual.value;

    if new_size <= size {
        // libc++ shrink is "move __end_ back". Capacity is untouched; reallocating to
        // fit would be a different vector.
        return OptVectorChar {
            vec: VectorChar {
                begin: begin.cast::<c_char>(),
                end: begin.add(new_size).cast::<c_char>(),
                cap: begin.add(max_size).cast::<c_char>(),
            },
            engaged: true,
        };
    }

    // Grow. Spare capacity is exactly zero here (capacity == size), so libc++ always
    // takes the reallocating branch of __append: a fresh buffer of __recommend(),
    // the existing `size` elements moved to the front, the rest value-initialised.
    //
    // The value-initialisation is why the byte that overran the old buffer never
    // surfaces: C++ copies only the first `size` bytes across and zero-fills the tail,
    // so the last byte of the result is 0 in both stacks regardless of what the
    // overrun wrote.
    let new_cap = vector_recommend(new_size, max_size);
    let nb = cxx_operator_new(new_cap);
    core::ptr::copy_nonoverlapping(begin, nb, size);
    core::ptr::write_bytes(nb.add(size), 0, new_size - size);
    cxx_operator_delete(begin);

    OptVectorChar {
        vec: VectorChar {
            begin: nb.cast::<c_char>(),
            end: nb.add(new_size).cast::<c_char>(),
            cap: nb.add(new_cap).cast::<c_char>(),
        },
        engaged: true,
    }
}

/// Extra bytes allocated for the decode buffer, over what the C++ allocates.
///
/// `base64_decode` can write exactly one byte more than
/// `base64_decoded_size(input.size())` reserves. It happens when the input is `4k`
/// valid characters followed by exactly one `=` and no whitespace: the four-character
/// groups emit `3k` bytes, the single `=` sends the tail through the
/// `num_trailing_equals == 1` branch which emits two more, giving `3k + 2`, while
/// `(3(4k+1) + 3) / 4` is `3k + 1`. Every other combination of valid characters,
/// padding and whitespace fits — whitespace only ever inflates the input length and
/// therefore the reservation. So the overrun is at most one byte, and `"="`,
/// `"AAAA="`, `"AAAAAAAA="` … are the inputs that reach it.
///
/// In C++ that is a one-byte heap overflow in `base64_decode_to_vector`, and the
/// `REALM_ASSERT_EX` that would have caught it is compiled out in Release. This port
/// reproduces the *observable* behaviour exactly — same returned size, capacity and
/// contents, verified by migration/checks/ — but allocates one spare byte so the
/// mirrored write stays inside its own allocation. The slack is never part of the
/// vector: `cap` is set to `max_size`, so C++ sees the capacity it expects, and
/// `operator delete` gets the pointer it was given.
///
/// This is the deliberate exception to "mirror the C++ even where it looks wrong".
/// That rule exists to protect on-disk layout, and no byte here reaches a file;
/// shipping a new heap overflow to honour it would be trading a real memory-safety
/// bug for nothing.
const DECODE_OVERRUN_SLACK: usize = 1;

/// Mirror of libc++'s `std::vector<char>::__recommend(new_size)`.
///
/// Not `max(2*cap, new_size)` alone: the `__ms` clamp changes the answer for
/// capacities above half of `max_size()`, and `max_size()` for `vector<char>` with
/// `std::allocator` is `PTRDIFF_MAX`.
#[inline]
fn vector_recommend(new_size: usize, cap: usize) -> usize {
    const MS: usize = isize::MAX as usize; // vector<char>::max_size()
    debug_assert!(new_size <= MS); // C++ throws std::length_error here
    if cap >= MS / 2 {
        return MS;
    }
    let doubled = 2 * cap;
    if doubled > new_size {
        doubled
    } else {
        new_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(input: &[u8]) -> String {
        let mut out = vec![0u8; encoded_size(input.len())];
        let n = unsafe {
            base64_encode(
                SpanConstChar {
                    data: input.as_ptr().cast(),
                    size: input.len(),
                },
                SpanChar {
                    data: out.as_mut_ptr().cast(),
                    size: out.len(),
                },
            )
        };
        assert_eq!(n, out.len());
        String::from_utf8(out).unwrap()
    }

    fn dec(input: &str) -> Option<Vec<u8>> {
        let mut out = vec![0u8; decoded_size(input.len())];
        let r = unsafe {
            base64_decode(
                SpanConstChar {
                    data: input.as_ptr().cast(),
                    size: input.len(),
                },
                SpanChar {
                    data: out.as_mut_ptr().cast(),
                    size: out.len(),
                },
            )
        };
        if !r.engaged {
            return None;
        }
        out.truncate(r.value);
        Some(out)
    }

    #[test]
    fn rfc4648_vectors() {
        assert_eq!(enc(b""), "");
        assert_eq!(enc(b"f"), "Zg==");
        assert_eq!(enc(b"fo"), "Zm8=");
        assert_eq!(enc(b"foo"), "Zm9v");
        assert_eq!(enc(b"foob"), "Zm9vYg==");
        assert_eq!(enc(b"fooba"), "Zm9vYmE=");
        assert_eq!(enc(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn round_trip_all_lengths() {
        for len in 0..200usize {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(dec(&enc(&data)).as_deref(), Some(&data[..]), "len {len}");
        }
    }

    #[test]
    fn encoder_covers_the_whole_alphabet() {
        // Every 6-bit value must be reachable, or the table was mistranscribed.
        let all: Vec<u8> = (0..=255u8).collect();
        let e = enc(&all);
        for c in ENCODING_CHARS.iter() {
            assert!(e.contains(*c as char), "alphabet char {} missing", *c as char);
        }
    }

    #[test]
    fn whitespace_set_is_tab_lf_space_but_not_cr() {
        // The C++ table marks 0x0D as invalid. Preserved deliberately: "fixing" it
        // would make the Rust accept input the C++ rejects.
        assert_eq!(dec("Zm9v\tYmFy"), Some(b"foobar".to_vec()));
        assert_eq!(dec("Zm9v\nYmFy"), Some(b"foobar".to_vec()));
        assert_eq!(dec("Zm9v YmFy"), Some(b"foobar".to_vec()));
        assert_eq!(dec("Zm9v\rYmFy"), None);
    }

    #[test]
    fn url_safe_alphabet_is_accepted_on_decode() {
        // '-' -> 62 and '_' -> 63, same as '+' and '/'.
        assert_eq!(dec("-_8="), dec("+/8="));
        assert!(dec("-_8=").is_some());
    }

    #[test]
    fn rejects_data_after_padding_and_bad_chars() {
        assert_eq!(dec("Zg==Zg=="), None);
        assert_eq!(dec("Zm9*"), None);
        assert_eq!(dec("Zg==="), None); // 3 trailing '=' falls through to the else
    }

    #[test]
    fn unpadded_input_is_inferred_from_length() {
        // extra == 2 or 3 synthesises the padding count; extra == 1 leaves
        // num_trailing_equals at 0 with a non-empty buffer, which is rejected.
        assert_eq!(dec("Zg"), Some(b"f".to_vec()));
        assert_eq!(dec("Zm8"), Some(b"fo".to_vec()));
        assert_eq!(dec("Z"), None);
    }

    #[test]
    fn size_helpers_match_the_header() {
        for n in 0..64usize {
            assert_eq!(encoded_size(n), 4 * ((n + 2) / 3));
            assert_eq!(decoded_size(n), (n * 3 + 3) / 4);
        }
    }
}
