//! `realm_binary64_to_bid128` — a mirror of `upstream/src/realm/decimal128.cpp:434-1353`.
//!
//! # Why this exists at all
//!
//! realm vendored Intel's `binary64_to_bid128` with all macros expanded, to avoid pulling
//! `bid_binarydecimal.c` and ~2MB into the binary (`decimal128.cpp:1352`). The consequence
//! is that Rust cannot call it: `binary64_to_bid128` is defined nowhere in this build, and
//! `realm_binary64_to_bid128` is in an anonymous namespace and was inlined into its only
//! caller at `-O3`, so it has no symbol at all. It has to be reimplemented.
//!
//! # What is mirrored and what is not
//!
//! The control flow, the constants, the branch conditions and the exact carry expressions
//! are transcribed one-for-one. What is *not* transcribed line-for-line is the
//! multiply macros: `__mul_64x64_to_128` and friends are expanded into ~700 of the 919
//! lines, and they are exact schoolbook multiplication with no truncation anywhere. They
//! are written here as helper functions instead. `mul_64x64_to_128` uses `u128`, which
//! computes the same function as the macro's four 32-bit partial products; the rest keep
//! the macro's carry expressions verbatim, including the shape
//! `(out < x1) || (x1 < carry)`, because that is where a rewrite would go wrong.
//!
//! This is a reduction in *transcription surface*, not a re-derivation of the algorithm.
//! `migration/checks/decimal128/README.md` records why the algorithm itself is mirrored
//! rather than re-derived from the definition of a binary64.
//!
//! # Arithmetic
//!
//! Every operation here is `wrapping_*`. The C is compiled at `-O3 -DNDEBUG` with
//! `REALM_ENABLE_ASSERTIONS=OFF`, so it wraps freely; this workspace sets
//! `overflow-checks = true`, which would panic instead. Two places genuinely rely on it:
//! the `t_prime.w[4] + 1` reload below, and the `×10` correction loop.

use super::tables::{
    BID_COEFFLIMITS_BID128, BID_INNERTABLE_EXP, BID_INNERTABLE_SIG, BID_OUTERTABLE_EXP,
    BID_OUTERTABLE_SIG, BID_POWER_FIVE, BID_ROUNDBOUND_128,
};

extern "C" {
    /// Intel's global rounding mode, read by the rounding step. Defined in the BID
    /// objects inside `librealm.a` (`___bid_IDEC_glbround` in `nm`), so this binds rather
    /// than duplicating it — the oracle and the hybrid must observe the *same* variable,
    /// not two copies that could drift.
    #[link_name = "__bid_IDEC_glbround"]
    static BID_IDEC_GLBROUND: u32;
}

/// `pfpsf` flag bits, from `bid_functions.h`. Only these three are ever set here.
const FLAG_INVALID: u32 = 0x01;
const FLAG_UNDERFLOW: u32 = 0x02;
const FLAG_INEXACT: u32 = 0x20;

// ---------------------------------------------------------------------------
// Multiply helpers. See the module comment: these replace macro expansions, and each
// keeps the original's carry expressions.
// ---------------------------------------------------------------------------

/// `__mul_64x64_to_128`. The macro forms four 32×32 partial products and reassembles
/// them with exact carries; that is precisely a 64×64→128 multiply.
#[inline]
fn mul_64x64_to_128(x: u64, y: u64) -> [u64; 2] {
    let p = (x as u128).wrapping_mul(y as u128);
    [p as u64, (p >> 64) as u64]
}

/// `__mul_64x256_to_320`.
#[inline]
fn mul_64x256_to_320(x: u64, y: &[u64; 4]) -> [u64; 5] {
    let lp0 = mul_64x64_to_128(x, y[0]);
    let lp1 = mul_64x64_to_128(x, y[1]);
    let lp2 = mul_64x64_to_128(x, y[2]);
    let lp3 = mul_64x64_to_128(x, y[3]);

    let mut p = [0u64; 5];
    p[0] = lp0[0];

    // `X1 = lP1.w[0]; P.w[1] = lP1.w[0] + lP0.w[1]; lC = (P.w[1] < X1)`
    let x1 = lp1[0];
    p[1] = lp1[0].wrapping_add(lp0[1]);
    let mut lc = (p[1] < x1) as u64;

    // The remaining three steps share one shape:
    //   X1 = lP{n}.w[0] + lC; P.w[n] = X1 + lP{n-1}.w[1];
    //   lC = (P.w[n] < X1) || (X1 < lC)
    for (n, (lo, hi)) in [(lp2[0], lp1[1]), (lp3[0], lp2[1])].iter().enumerate() {
        let x1 = lo.wrapping_add(lc);
        p[n + 2] = x1.wrapping_add(*hi);
        lc = ((p[n + 2] < x1) || (x1 < lc)) as u64;
    }
    p[4] = lp3[1].wrapping_add(lc);
    p
}

/// Add a 320-bit row into an accumulator at a 64-bit word offset, returning the carry-out
/// word. This is the `{ X1 = P.w[i] + CY; acc.w[j] = X1 + acc.w[j]; ... }` chain that the
/// 384- and 512-bit products both repeat.
#[inline]
fn accumulate_row(acc: &mut [u64], row: &[u64; 5], offset: usize) -> u64 {
    let x1 = row[0];
    acc[offset] = row[0].wrapping_add(acc[offset]);
    let mut cy = (acc[offset] < x1) as u64;
    for i in 1..4 {
        let x1 = row[i].wrapping_add(cy);
        acc[offset + i] = x1.wrapping_add(acc[offset + i]);
        cy = ((acc[offset + i] < x1) || (x1 < cy)) as u64;
    }
    row[4].wrapping_add(cy)
}

/// `__mul_128x256_to_384`: `z = x * y`, with `x` the 128-bit operand.
#[inline]
fn mul_128x256_to_384(x: &[u64; 2], y: &[u64; 4]) -> [u64; 6] {
    let p0 = mul_64x256_to_320(x[0], y);
    let p1 = mul_64x256_to_320(x[1], y);

    let mut z = [0u64; 6];
    z[0] = p0[0];
    let x1 = p1[0];
    z[1] = p1[0].wrapping_add(p0[1]);
    let mut cy = (z[1] < x1) as u64;
    for i in 1..4 {
        let x1 = p1[i].wrapping_add(cy);
        z[i + 1] = x1.wrapping_add(p0[i + 1]);
        cy = ((z[i + 1] < x1) || (x1 < cy)) as u64;
    }
    z[5] = p1[4].wrapping_add(cy);
    z
}

/// `__mul_256x256_to_512`.
#[inline]
fn mul_256x256_to_512(x: &[u64; 4], y: &[u64; 4]) -> [u64; 8] {
    let p0 = mul_64x256_to_320(x[0], y);
    let p1 = mul_64x256_to_320(x[1], y);
    let p2 = mul_64x256_to_320(x[2], y);
    let p3 = mul_64x256_to_320(x[3], y);

    let mut t = [0u64; 8];
    // Rows 0 and 1 combine exactly as in mul_128x256_to_384.
    t[0] = p0[0];
    let x1 = p1[0];
    t[1] = p1[0].wrapping_add(p0[1]);
    let mut cy = (t[1] < x1) as u64;
    for i in 1..4 {
        let x1 = p1[i].wrapping_add(cy);
        t[i + 1] = x1.wrapping_add(p0[i + 1]);
        cy = ((t[i + 1] < x1) || (x1 < cy)) as u64;
    }
    t[5] = p1[4].wrapping_add(cy);

    t[6] = accumulate_row(&mut t, &p2, 2);
    t[7] = accumulate_row(&mut t, &p3, 3);
    t
}

/// The 128-bit right shift the two fast paths use, written as the C ternary chain writes
/// it — including that a shift of 0 is a no-op and a shift of >= 64 clears the high word.
#[inline]
fn shr_128(c: &mut [u64; 2], n: i32) {
    if n == 0 {
        // no-op, as in the C
    } else if n >= 64 {
        c[0] = c[1] >> (n - 64);
        c[1] = 0;
    } else {
        c[0] = (c[1] << (64 - n)) | (c[0] >> n);
        c[1] >>= n;
    }
}

/// `x < y` on 128-bit values held as `[lo, hi]`.
#[inline]
fn lt_128(xl: u64, xh: u64, yl: u64, yh: u64) -> bool {
    xh < yh || (xh == yh && xl < yl)
}

// ---------------------------------------------------------------------------
// The conversion
// ---------------------------------------------------------------------------

/// Mirror of `realm_binary64_to_bid128(BID_UINT128* pres, double* px, _IDEC_flags* pfpsf)`.
///
/// Returns the 128-bit result as `[lo, hi]` and updates `flags` in place, exactly as the
/// C writes through `pfpsf`. The flags are an output: a caller that ignores them is
/// discarding half of what this function computes.
pub(crate) fn binary64_to_bid128(x: f64, flags: &mut u32) -> [u64; 2] {
    let mut c = [0u64; 2];

    // --- Unpack the input -------------------------------------------------
    let bits = x.to_bits();
    c[1] = bits;
    let mut e: i32 = ((c[1] >> 52) & ((1u64 << 11) - 1)) as i32;
    let s: u64 = c[1] >> 63;
    c[1] &= (1u64 << 52) - 1;
    let t: i32;

    if e == 0 {
        if c[1] == 0 {
            // Signed zero.
            return [0, (s << 63).wrapping_add(6176u64 << 49)];
        }
        // The C expands a leading-zero count here; `leading_zeros` agrees on the whole
        // domain, including the zero case (both give 64), which the branch above has
        // already excluded anyway.
        let l = c[1].leading_zeros() as i32 - (64 - 53);
        c[1] <<= l;
        e = -(l + 1074);
        t = 0;
        *flags |= FLAG_UNDERFLOW;
    } else if e == (1 << 11) - 1 {
        if c[1] == 0 {
            // Signed infinity.
            return [0, (s << 63).wrapping_add(((0xF << 10) as u64) << 49)];
        }
        if (c[1] & (1u64 << 51)) == 0 {
            *flags |= FLAG_INVALID;
        }
        // NaN payload: shifted left 13 then split at bit 18.
        let payload_hi = (c[1] << 13) >> 18;
        let payload_lo = (c[1] << 13) << 46;
        if 54210108624275u64 < payload_hi
            || (54210108624275u64 == payload_hi && 4089650035136921599u64 < payload_lo)
        {
            // Payload out of range: drop it.
            return [0, (s << 63).wrapping_add(((0x1F << 9) as u64) << 49)];
        }
        return [
            payload_lo,
            (s << 63)
                .wrapping_add(((0x1F << 9) as u64) << 49)
                .wrapping_add(payload_hi),
        ];
    } else {
        c[1] += 1u64 << 52;
        // The C expands a trailing-zero count; `trailing_zeros` agrees, including the
        // zero case (both 64), which cannot arise here because of the implicit bit.
        t = c[1].trailing_zeros() as i32;
        e -= 1075;
    }

    // Shift up to the top: a pure quad coefficient with a shift of 15, so unpack at the
    // high end shifted by 11.
    c[0] = 0;
    c[1] <<= 11;
    let t = t + (113 - 53);
    e -= 113 - 53;

    // --- Exact cases that must force the exponent to 0 --------------------
    if e <= 0 {
        let a = -(e + t);
        let mut cint = c;
        if a <= 0 {
            shr_128(&mut cint, 15 - e);
            if lt_128(cint[0], cint[1], 4003012203950112768, 542101086242752) {
                return [
                    cint[0],
                    (s << 63).wrapping_add(6176u64 << 49).wrapping_add(cint[1]),
                ];
            }
        } else if a <= 48 {
            let pow5 = BID_COEFFLIMITS_BID128[a as usize];
            shr_128(&mut cint, 15 + t);
            // Note `<=` here, against `<` in the a <= 0 branch above. Mirrored.
            if cint[1] < pow5[1] || (cint[1] == pow5[1] && cint[0] <= pow5[0]) {
                let pow5 = BID_POWER_FIVE[a as usize];
                // __mul_128x128_low: the high 128 bits of the product are discarded.
                let albl = mul_64x64_to_128(cint[0], pow5[0]);
                let qm64 = pow5[0]
                    .wrapping_mul(cint[1])
                    .wrapping_add(cint[0].wrapping_mul(pow5[1]));
                let cc = [albl[0], qm64.wrapping_add(albl[1])];
                return [
                    cc[0],
                    (s << 63)
                        .wrapping_add(((6176 - a) as u64) << 49)
                        .wrapping_add(cc[1]),
                ];
            }
        }
    }

    // --- Estimate the decimal exponent ------------------------------------
    let e_plus = e + 42152;
    let mut e_out = (((19728 * e_plus) + ((19779 * e_plus) >> 16)) >> 16) - 6512;

    let mut e_hi = 11232 - e_out;
    let e_lo = e_hi & 127;
    e_hi >>= 7;

    let mut r = BID_INNERTABLE_SIG[e_lo as usize];
    let mut f = BID_INNERTABLE_EXP[e_lo as usize];

    if e_hi != 39 {
        let s_prime = BID_OUTERTABLE_SIG[e_hi as usize];
        f = f + 256 + BID_OUTERTABLE_EXP[e_hi as usize];
        let t_prime = mul_256x256_to_512(&r, &s_prime);
        // Take the top 256 bits, and add one to the lowest word of *that* -- with no
        // carry into the words above it. Mirrored exactly; a carrying increment here
        // would be a different function.
        r = [
            t_prime[4].wrapping_add(1),
            t_prime[5],
            t_prime[6],
            t_prime[7],
        ];
    }

    let mut z = mul_128x256_to_384(&c, &r);

    // --- Adjustive shift, ignoring the lower 128 bits ---------------------
    let e = -(241 + e + f);
    // The C indexes one word past the end for z[5] and shifts by `64 - e`; for the range
    // Intel guarantees here (1..=63) that is well defined, and `wrapping_sh*` reproduces
    // what the compiled code does if it ever were not.
    let sh = e as u32;
    for i in 0..5 {
        z[i] = (z[i + 1].wrapping_shl(64 - sh)).wrapping_add(z[i].wrapping_shr(sh));
    }
    z[5] = z[5].wrapping_shr(sh);

    // --- Test against 10^33 and decide on adjustment ----------------------
    if z[5] < 54210108624275 || (z[5] == 54210108624275 && z[4] < 4089650035136921600) {
        // Multiply the 384-bit z by ten, word by word:
        //   s3 = x + (x >> 2);  carry = ((s3 < x) << 3) + (s3 >> 61)
        //   s3 = (s3 << 3) + ((x & 3) << 1);  x = s3 + carry_in
        // i.e. 10x == ((x + x/4) * 8) + (x & 3) * 2, with the overflow of the first add
        // contributing 8 to the next word.
        let mut carry = 0u64;
        for i in 0..6 {
            let x = z[i];
            let mut s3 = x.wrapping_add(x >> 2);
            let mut c_i = (((s3 < x) as u64) << 3).wrapping_add(s3 >> 61);
            s3 = (s3 << 3).wrapping_add((x & 3) << 1);
            z[i] = s3.wrapping_add(carry);
            if z[i] < s3 {
                c_i = c_i.wrapping_add(1);
            }
            carry = c_i;
        }
        e_out -= 1;
    }

    // --- Provisional result and round-sticky rounding ---------------------
    let mut c_prov_hi = z[5];
    let mut c_prov_lo = z[4];

    let glbround = unsafe { BID_IDEC_GLBROUND };
    let bound = BID_ROUNDBOUND_128
        [((glbround << 2) as u64 + ((s & 1) << 1) + (c_prov_lo & 1)) as usize];
    if bound[1] < z[3] || (bound[1] == z[3] && bound[0] < z[2]) {
        c_prov_lo = c_prov_lo.wrapping_add(1);
        if c_prov_lo == 0 {
            c_prov_hi = c_prov_hi.wrapping_add(1);
        } else if c_prov_lo == 4003012203950112768 && c_prov_hi == 542101086242752 {
            // Spilled into the next decade.
            c_prov_hi = 54210108624275;
            c_prov_lo = 4089650035136921600;
            e_out += 1;
        }
    }

    // No overflow or underflow check is needed here -- this is the widest format -- but
    // inexact must be reported.
    if z[3] != 0 || z[2] != 0 {
        *flags |= FLAG_INEXACT;
    }

    [
        c_prov_lo,
        (s << 63)
            .wrapping_add((e_out as u64) << 49)
            .wrapping_add(c_prov_hi),
    ]
}
