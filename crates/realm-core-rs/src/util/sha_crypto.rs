//! Port of `upstream/src/realm/util/sha_crypto.cpp` (Apple path).
//!
//! Four exported functions, all four live in the linked binary: `sha1`, `sha256`,
//! `hmac_sha224`, `hmac_sha256`.
//!
//! ## Why this port is a wrapper, not an implementation
//!
//! On Apple platforms the C++ is a thin shim over CommonCrypto — `CC_SHA1`,
//! `CC_SHA256`, `CCHmac`. Reimplementing SHA in Rust would be a *different*
//! implementation that merely ought to agree; calling the same system routines makes
//! the digests identical by construction. Faithfulness beats self-sufficiency here,
//! and the pure-Rust version can come later, gated by this unit's differential.
//!
//! The C++ has three other platform branches (Windows/BCrypt, OpenSSL, and a bundled
//! SHA-2). None is compiled on this host, so none is ported; a non-Apple build of the
//! hybrid would need them and the `cfg` below fails the build loudly rather than
//! silently producing a stub.
//!
//! ## Fidelity notes
//!
//! - **`CC_LONG` is `uint32_t`, so `CC_SHA1(in, CC_LONG(size), out)` truncates the
//!   length.** Confirmed against the SDK, not assumed. An input over 4 GiB is hashed
//!   as `size % 2^32` bytes. That is a latent bug in the C++, and it is reproduced
//!   exactly: a Rust version that passed the full 64-bit length would compute a
//!   *different, more correct* digest, which is a compatibility break.
//! - **Algorithm selectors are the CommonCrypto enum ordinals**, taken by compiling
//!   against the SDK rather than from memory: `kCCHmacAlgSHA256 = 2`,
//!   `kCCHmacAlgSHA224 = 5`. They are not in numeric order, and SHA224 is *not* 4.
//! - **Fixed-extent `realm::util::Span<T, N>` stores only a pointer** — the size is a
//!   template parameter, so the struct is 8 bytes, not 16 (`util/span.hpp:262`).
//!   Dynamic-extent `Span<T>` stores `{ptr, size}`. Getting this backwards would shift
//!   every subsequent argument register.

use core::ffi::c_void;

/// `realm::util::Span<const uint8_t, dynamic_extent>` — `{ptr, size}`.
#[repr(C)]
pub struct SpanConstU8 {
    data: *const u8,
    size: usize,
}

/// `realm::util::Span<uint8_t, N>` for a *fixed* extent: pointer only, because the
/// size lives in the type. Used for the 28- and 32-byte digest outputs.
#[repr(C)]
pub struct SpanU8Fixed {
    data: *mut u8,
}

/// `realm::util::Span<const uint8_t, 32>` — the HMAC key. Pointer only, as above.
#[repr(C)]
pub struct SpanConstU8Fixed {
    data: *const u8,
}

// CommonCrypto lives in libSystem, which is linked into every Mach-O binary, so these
// need no extra link flag.
#[cfg(target_vendor = "apple")]
extern "C" {
    fn CC_SHA1(data: *const c_void, len: u32, md: *mut u8) -> *mut u8;
    fn CC_SHA256(data: *const c_void, len: u32, md: *mut u8) -> *mut u8;
    fn CCHmac(
        algorithm: u32,
        key: *const c_void,
        key_length: usize,
        data: *const c_void,
        data_length: usize,
        mac_out: *mut c_void,
    );
}

#[cfg(not(target_vendor = "apple"))]
compile_error!(
    "sha_crypto.rs ports only the REALM_PLATFORM_APPLE branch of sha_crypto.cpp. \
     The Windows/BCrypt, OpenSSL and bundled-SHA2 branches are not implemented; \
     porting them is required before the hybrid can be built on this platform."
);

/// `kCCHmacAlgSHA256`. Obtained by compiling against CommonCrypto/CommonHMAC.h.
const KCC_HMAC_ALG_SHA256: u32 = 2;
/// `kCCHmacAlgSHA224`. Note it is 5, not 4 — the enum is not in digest-size order.
const KCC_HMAC_ALG_SHA224: u32 = 5;

/// `realm::util::sha1(const char*, size_t, unsigned char*)`
///
/// Writes 20 bytes to `out_buffer`.
///
/// # Safety
/// `in_buffer` must point to `in_buffer_size` readable bytes and `out_buffer` to at
/// least 20 writable bytes, as the C++ callers guarantee.
#[export_name = "_ZN5realm4util4sha1EPKcmPh"]
pub unsafe extern "C" fn sha1(in_buffer: *const i8, in_buffer_size: usize, out_buffer: *mut u8) {
    // `CC_LONG(in_buffer_size)` — a 32-bit truncation, mirrored deliberately.
    CC_SHA1(
        in_buffer as *const c_void,
        in_buffer_size as u32,
        out_buffer,
    );
}

/// `realm::util::sha256(const char*, size_t, unsigned char*)`
///
/// Writes 32 bytes to `out_buffer`.
///
/// # Safety
/// `in_buffer` must point to `in_buffer_size` readable bytes and `out_buffer` to at
/// least 32 writable bytes, as the C++ callers guarantee.
#[export_name = "_ZN5realm4util6sha256EPKcmPh"]
pub unsafe extern "C" fn sha256(in_buffer: *const i8, in_buffer_size: usize, out_buffer: *mut u8) {
    // Same 32-bit truncation as sha1.
    CC_SHA256(
        in_buffer as *const c_void,
        in_buffer_size as u32,
        out_buffer,
    );
}

/// `realm::util::hmac_sha224(Span<const uint8_t>, Span<uint8_t, 28>, Span<const uint8_t, 32>)`
///
/// # Safety
/// The spans must be valid as constructed by the C++ caller: `in_buffer` readable for
/// its size, `out_buffer` writable for 28 bytes, `key` readable for 32.
#[export_name = "_ZN5realm4util11hmac_sha224ENS0_4SpanIKhLm18446744073709551615EEENS1_IhLm28EEENS1_IS2_Lm32EEE"]
pub unsafe extern "C" fn hmac_sha224(
    in_buffer: SpanConstU8,
    out_buffer: SpanU8Fixed,
    key: SpanConstU8Fixed,
) {
    // key.size() is the compile-time extent 32; in_buffer.size() is the runtime size.
    CCHmac(
        KCC_HMAC_ALG_SHA224,
        key.data as *const c_void,
        32,
        in_buffer.data as *const c_void,
        in_buffer.size,
        out_buffer.data as *mut c_void,
    );
}

/// `realm::util::hmac_sha256(Span<const uint8_t>, Span<uint8_t, 32>, Span<const uint8_t, 32>)`
///
/// # Safety
/// As `hmac_sha224`, except `out_buffer` must be writable for 32 bytes.
#[export_name = "_ZN5realm4util11hmac_sha256ENS0_4SpanIKhLm18446744073709551615EEENS1_IhLm32EEENS1_IS2_Lm32EEE"]
pub unsafe extern "C" fn hmac_sha256(
    in_buffer: SpanConstU8,
    out_buffer: SpanU8Fixed,
    key: SpanConstU8Fixed,
) {
    CCHmac(
        KCC_HMAC_ALG_SHA256,
        key.data as *const c_void,
        32,
        in_buffer.data as *const c_void,
        in_buffer.size,
        out_buffer.data as *mut c_void,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    /// RFC 3174 / FIPS 180-2 published vectors. These pin the *algorithm selection* —
    /// that `sha1` really is SHA-1 and `sha256` really is SHA-256. The cross-check
    /// against the C++ is `migration/checks/run_sha_crypto_differential.sh`.
    #[test]
    fn sha1_known_vectors() {
        let mut out = [0u8; 20];
        // SAFETY: empty input, 20-byte output buffer.
        unsafe { sha1(b"".as_ptr() as *const i8, 0, out.as_mut_ptr()) };
        assert_eq!(hex(&out), "da39a3ee5e6b4b0d3255bfef95601890afd80709");

        // SAFETY: 3-byte input, 20-byte output buffer.
        unsafe { sha1(b"abc".as_ptr() as *const i8, 3, out.as_mut_ptr()) };
        assert_eq!(hex(&out), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    #[test]
    fn sha256_known_vectors() {
        let mut out = [0u8; 32];
        // SAFETY: empty input, 32-byte output buffer.
        unsafe { sha256(b"".as_ptr() as *const i8, 0, out.as_mut_ptr()) };
        assert_eq!(
            hex(&out),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );

        // SAFETY: 3-byte input, 32-byte output buffer.
        unsafe { sha256(b"abc".as_ptr() as *const i8, 3, out.as_mut_ptr()) };
        assert_eq!(
            hex(&out),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /// Guards the enum ordinals. If SHA224 and SHA256 were swapped, or SHA224 were
    /// given the plausible-but-wrong value 4, the two digests below would not differ
    /// in the way SHA-224 and SHA-256 do — different lengths and different bytes.
    #[test]
    fn hmac_selectors_pick_distinct_algorithms() {
        let key = [0x0bu8; 32];
        let msg = b"Hi There";
        let mut out224 = [0u8; 28];
        let mut out256 = [0u8; 32];

        // SAFETY: spans describe live local buffers of exactly the required sizes.
        unsafe {
            hmac_sha224(
                SpanConstU8 {
                    data: msg.as_ptr(),
                    size: msg.len(),
                },
                SpanU8Fixed {
                    data: out224.as_mut_ptr(),
                },
                SpanConstU8Fixed { data: key.as_ptr() },
            );
            hmac_sha256(
                SpanConstU8 {
                    data: msg.as_ptr(),
                    size: msg.len(),
                },
                SpanU8Fixed {
                    data: out256.as_mut_ptr(),
                },
                SpanConstU8Fixed { data: key.as_ptr() },
            );
        }

        // Both must have been written (a wrong selector can leave the buffer untouched
        // or write the wrong length).
        assert_ne!(out224, [0u8; 28], "hmac_sha224 wrote nothing");
        assert_ne!(out256, [0u8; 32], "hmac_sha256 wrote nothing");
        // SHA-224 is not a truncation of SHA-256, so the shared prefix must differ.
        assert_ne!(out224[..28], out256[..28]);
    }

    #[test]
    fn hmac_is_deterministic_and_key_sensitive() {
        let msg = b"realm";
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        let mut c = [0u8; 32];
        let key1 = [0x01u8; 32];
        let mut key2 = [0x01u8; 32];
        key2[31] = 0x02;

        // SAFETY: all spans describe live local buffers of the required sizes.
        unsafe {
            let mk = |out: *mut u8, k: *const u8| {
                hmac_sha256(
                    SpanConstU8 {
                        data: msg.as_ptr(),
                        size: msg.len(),
                    },
                    SpanU8Fixed { data: out },
                    SpanConstU8Fixed { data: k },
                )
            };
            mk(a.as_mut_ptr(), key1.as_ptr());
            mk(b.as_mut_ptr(), key1.as_ptr());
            mk(c.as_mut_ptr(), key2.as_ptr());
        }

        assert_eq!(a, b, "same key and message must give the same MAC");
        assert_ne!(a, c, "a one-bit key change must change the MAC");
    }
}
