//! `upstream/src/realm/decimal128.cpp` — `realm::Decimal128`.
//!
//! The unit splits cleanly in two. [`conv`] holds the 919-line mirror of
//! `realm_binary64_to_bid128`, which had to be reimplemented because it has no symbol to
//! link (anonymous namespace, inlined at `-O3`) and the Intel library's own
//! `binary64_to_bid128` is absent from this build. Everything here is the other half:
//! thin wrappers over BID entry points that *are* linkable.
//!
//! # Layout
//!
//! `Decimal128` is one `Bid128`, i.e. `uint64_t w[2]` — 16 bytes, no vptr, trivially
//! copyable. Under the SysV ABI that is INTEGER,INTEGER: passed and returned in two
//! registers, never `sret`. Confirmed on the oracle rather than assumed:
//!
//! ```text
//! Decimal128::operator*(Decimal128) const
//!   1af8: movups (%rdi), %xmm0      ; rdi = this, so no sret
//!   1aff: movq   %rsi, -0x30(%rbp)  ; rsi:rdx = the by-value argument
//!
//! Decimal128::Decimal128(Bid128, int, bool)
//!   14d8: movq %rsi, (%rdi)         ; rsi:rdx = Bid128
//!   14db: addl $0x1820, %ecx        ; ecx = exponent  (0x1820 == 6176, the bias)
//!   14d4: shlq $0x3f, %r8           ; r8  = sign
//! ```
//!
//! `std::optional<Bid32>` (8 bytes) and `std::optional<Bid64>` (16 bytes) are both
//! trivially copyable, so both come back in registers too — `to_bid32` and `to_bid64`
//! each start by dereferencing `rdi` as `this`. Only `to_string` uses `sret`.
//!
//! # Constructor variants
//!
//! The Itanium ABI emits `C1` (complete) and `C2` (base) for every constructor and the
//! compiler emits both even when identical. Each is defined here as a thin forwarder to
//! one shared body, per `unit-screening.md` step 4.

pub mod conv;
pub mod tables;

use core::ffi::{c_char, c_int, c_uint};

// ---------------------------------------------------------------------------
// Types crossing the boundary
// ---------------------------------------------------------------------------

/// `realm::Decimal128` — and `Decimal128::Bid128`, which is the same 16 bytes.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Decimal128 {
    pub w: [u64; 2],
}

/// `realm::StringData` — `{const char*, size_t}`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct StringData {
    data: *const c_char,
    size: usize,
}

/// `std::optional<Bid32>`: `uint32_t` value then the engaged flag. 8 bytes.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct OptBid32 {
    value: u32,
    engaged: bool,
}

/// `std::optional<Bid64>`: `uint64_t` value then the engaged flag, padded to 16.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct OptBid64 {
    value: u64,
    engaged: bool,
}

/// libc++ `std::string`, 24 bytes. Same representation `unicode.rs` and `status.rs` use.
#[repr(C, align(8))]
pub struct StdString {
    rep: [u8; 24],
}

/// `realm::util::Printable` — measured 24 bytes: the `Type` enum at 0, four bytes of
/// padding, then the union at 8. Only the two integer arms are needed here.
#[repr(C)]
struct Printable {
    ty: u32,
    _pad: u32,
    val: [u64; 2],
}

const PRINTABLE_INT: u32 = 1;
const PRINTABLE_UINT: u32 = 2;

const _: () = {
    assert!(core::mem::size_of::<Decimal128>() == 16);
    assert!(core::mem::size_of::<OptBid32>() == 8);
    assert!(core::mem::size_of::<OptBid64>() == 16);
    assert!(core::mem::size_of::<StdString>() == 24);
    assert!(core::mem::size_of::<Printable>() == 24);
};

// ---------------------------------------------------------------------------
// Constants, from decimal128.hpp
// ---------------------------------------------------------------------------

const DECIMAL_EXPONENT_BIAS_128: i32 = 6176;
const DECIMAL_COEFF_HIGH_BITS: u32 = 49;
const DECIMAL_EXP_BITS: u32 = 14;
const DECIMAL_NULL_32: u32 = 0x7c00_00aa;
const DECIMAL_NULL_64: u64 = 0x7c00_0000_0000_00aa;
const NULL_LO: u64 = 0xaa;
const NULL_HI: u64 = 0x7c00_0000_0000_0000;
const MASK_COEFF: u64 = (1u64 << DECIMAL_COEFF_HIGH_BITS) - 1;
const MASK_EXP: u64 = ((1u64 << DECIMAL_EXP_BITS) - 1) << DECIMAL_COEFF_HIGH_BITS;
const MASK_SIGN: u64 = 1u64 << (DECIMAL_COEFF_HIGH_BITS + DECIMAL_EXP_BITS);

/// `DEC_FE_INEXACT`, `bid_functions.h:175`.
const BID_INEXACT_EXCEPTION: c_uint = 0x20;

/// `Decimal128::DECIMAL_NULL_128` — a `static constexpr` data member, and one of the
/// unit's 48 exported symbols. It has to be *defined*, not just used.
#[no_mangle]
pub static _ZN5realm10Decimal12816DECIMAL_NULL_128E: [u64; 2] = [NULL_LO, NULL_HI];

// ---------------------------------------------------------------------------
// Bound C and C++ symbols
// ---------------------------------------------------------------------------

extern "C" {
    // Intel BID. Every one of these is already in librealm.a and in the linked oracle,
    // which is what makes the rest of this unit thin. `#[link_name]` gets one leading
    // underscore added on Mach-O, so "__bid128_add" here is "___bid128_add" in `nm`.
    #[link_name = "__bid128_from_uint64"]
    fn bid128_from_uint64(res: *mut Decimal128, x: *const u64);
    #[link_name = "__bid32_to_bid128"]
    fn bid32_to_bid128(res: *mut Decimal128, x: *const u32, flags: *mut c_uint);
    #[link_name = "__bid64_to_bid128"]
    fn bid64_to_bid128(res: *mut Decimal128, x: *const u64, flags: *mut c_uint);
    #[link_name = "__bid128_from_string"]
    fn bid128_from_string(res: *mut Decimal128, s: *mut c_char, flags: *mut c_uint);
    #[link_name = "__bid128_to_string"]
    fn bid128_to_string(buf: *mut c_char, x: *const Decimal128, flags: *mut c_uint);
    #[link_name = "__bid128_to_int64_int"]
    fn bid128_to_int64_int(res: *mut i64, x: *const Decimal128, flags: *mut c_uint);
    #[link_name = "__bid128_quiet_equal"]
    fn bid128_quiet_equal(res: *mut c_int, x: *const Decimal128, y: *const Decimal128, f: *mut c_uint);
    #[link_name = "__bid128_quiet_less"]
    fn bid128_quiet_less(res: *mut c_int, x: *const Decimal128, y: *const Decimal128, f: *mut c_uint);
    #[link_name = "__bid128_quiet_greater"]
    fn bid128_quiet_greater(res: *mut c_int, x: *const Decimal128, y: *const Decimal128, f: *mut c_uint);
    #[link_name = "__bid128_mul"]
    fn bid128_mul(res: *mut Decimal128, x: *const Decimal128, y: *const Decimal128, f: *mut c_uint);
    #[link_name = "__bid128_div"]
    fn bid128_div(res: *mut Decimal128, x: *const Decimal128, y: *const Decimal128, f: *mut c_uint);
    #[link_name = "__bid128_add"]
    fn bid128_add(res: *mut Decimal128, x: *const Decimal128, y: *const Decimal128, f: *mut c_uint);
    #[link_name = "__bid128_sub"]
    fn bid128_sub(res: *mut Decimal128, x: *const Decimal128, y: *const Decimal128, f: *mut c_uint);
    #[link_name = "__bid128_quantize"]
    fn bid128_quantize(res: *mut Decimal128, x: *const Decimal128, y: *const Decimal128, f: *mut c_uint);
    #[link_name = "__bid128_to_bid32"]
    fn bid128_to_bid32(res: *mut u32, x: *const Decimal128, f: *mut c_uint);
    #[link_name = "__bid128_to_bid64"]
    fn bid128_to_bid64(res: *mut u64, x: *const Decimal128, f: *mut c_uint);

    fn strtol(s: *const c_char, end: *mut *mut c_char, base: c_int) -> i64;
    fn frexp(x: f64, exp: *mut c_int) -> f64;

    /// `realm::util::Printable::str() const` — the unit's one realm-owned undefined
    /// symbol, external and present in the linked oracle. `util::to_string(v)` is
    /// literally `Printable(v).str()`, so binding this reproduces the digits rather than
    /// re-deriving integer formatting.
    #[link_name = "_ZNK5realm4util9Printable3strEv"]
    fn printable_str(sret: *mut StdString, this: *const Printable);

    /// `std::string::basic_string(const char*)`.
    ///
    /// Bound rather than reproduced. This is a *different* capacity rule from the
    /// append/growth path, and the difference is observable: for a 24-character result
    /// the constructor gives capacity 31 while growing an empty string by appending
    /// gives 47. The methods differential caught exactly that on its first run, on 17
    /// lines whose characters were identical.
    ///
    /// libc++'s measured constructor rule, for the record, is `n <= 22 -> 22`,
    /// `n == 23 -> 25`, `n >= 24 -> round_up(n + 1, 8) - 1`. It is recorded and *not*
    /// used, per the rule the unicode port established: a mirrored growth policy is a
    /// second copy of something that can drift with the toolchain, and a bound symbol
    /// cannot.
    #[link_name = "_ZNSt3__112basic_stringIcNS_11char_traitsIcEENS_9allocatorIcEEEC1B9nqe210106ILi0EEEPKc"]
    fn string_from_cstr(this: *mut StdString, s: *const c_char);

    #[link_name = "_ZNSt3__112basic_stringIcNS_11char_traitsIcEENS_9allocatorIcEEE6appendEPKcm"]
    fn string_append(this: *mut StdString, s: *const c_char, n: usize) -> *mut StdString;
    #[link_name = "_ZNSt3__112basic_stringIcNS_11char_traitsIcEENS_9allocatorIcEEE9push_backEc"]
    fn string_push_back(this: *mut StdString, c: c_char);
}

extern "C-unwind" {
    // Declared exactly as `unicode.rs` and `util/base64.rs` declare it, so the crate has
    // one signature for this symbol rather than two.
    #[link_name = "_ZdlPv"]
    fn cxx_operator_delete(p: *mut u8);
}

// ---------------------------------------------------------------------------
// std::string helpers (same shape as unicode.rs)
// ---------------------------------------------------------------------------

impl StdString {
    const EMPTY: StdString = StdString { rep: [0u8; 24] };

    #[inline]
    unsafe fn is_long(&self) -> bool {
        self.rep[0] & 1 != 0
    }

    #[inline]
    unsafe fn parts(&self) -> (*const c_char, usize) {
        let base = self.rep.as_ptr();
        if !self.is_long() {
            (base.add(1) as *const c_char, (*base >> 1) as usize)
        } else {
            let w = base as *const usize;
            (w.add(2).read_unaligned() as *const c_char, w.add(1).read_unaligned())
        }
    }

    #[inline]
    unsafe fn destroy(&mut self) {
        if self.is_long() {
            let w = self.rep.as_ptr() as *const usize;
            cxx_operator_delete(w.add(2).read_unaligned() as *mut u8);
        }
    }
}

/// `util::to_string(v)` == `Printable(v).str()`.
#[inline]
unsafe fn to_string_uint(v: u64) -> StdString {
    let p = Printable { ty: PRINTABLE_UINT, _pad: 0, val: [v, 0] };
    let mut out = core::mem::MaybeUninit::<StdString>::uninit();
    printable_str(out.as_mut_ptr(), &p);
    out.assume_init()
}

#[inline]
unsafe fn to_string_int(v: i32) -> StdString {
    let p = Printable { ty: PRINTABLE_INT, _pad: 0, val: [v as i64 as u64, 0] };
    let mut out = core::mem::MaybeUninit::<StdString>::uninit();
    printable_str(out.as_mut_ptr(), &p);
    out.assume_init()
}

// ---------------------------------------------------------------------------
// Shared bodies
// ---------------------------------------------------------------------------

#[inline]
fn is_null_val(v: &Decimal128) -> bool {
    v.w[0] == NULL_LO && v.w[1] == NULL_HI
}

#[inline]
fn is_nan_val(v: &Decimal128) -> bool {
    (v.w[1] & NULL_HI) == NULL_HI
}

#[inline]
fn coefficient_high(v: &Decimal128) -> u64 {
    v.w[1] & MASK_COEFF
}

unsafe fn ctor_from_i64(this: *mut Decimal128, val: i64) {
    let expon = (DECIMAL_EXPONENT_BIAS_128 as u64) << DECIMAL_COEFF_HIGH_BITS;
    if val < 0 {
        (*this).w[1] = expon | MASK_SIGN;
        // The C spells the negation as `val == lowest() ? val : ~val + 1`, which is the
        // same bit pattern as a wrapping negation -- INT64_MIN negates to itself. Kept
        // in the C's shape rather than folded to `wrapping_neg`, since that equivalence
        // is the sort of thing that is true until someone edits it.
        (*this).w[0] = if val == i64::MIN { val as u64 } else { (!(val as u64)).wrapping_add(1) };
    } else {
        (*this).w[1] = expon;
        (*this).w[0] = val as u64;
    }
}

unsafe fn ctor_from_double(this: *mut Decimal128, val: f64, rounding_precision: c_int) {
    // RoundTo::Digits7 == 0, RoundTo::Digits15 == 1
    let largest_coeff: u64 = if rounding_precision == 0 { 9_999_999 } else { 999_999_999_999_999 };
    let mut flags: u32 = 0;
    let converted = conv::binary64_to_bid128(val, &mut flags);
    (*this).w = converted;

    if ((*this).w[0] <= largest_coeff && coefficient_high(&*this) == 0)
        || val.is_infinite()
        || val.is_nan()
    {
        return;
    }

    let mut base2_exp: c_int = 0;
    frexp(val, &mut base2_exp);
    // frexp normalises to [0.5, 1.0) rather than [1.0, 2.0).
    base2_exp -= 1;

    let mut base10_exp = (base2_exp * 30103) / (100 * 1000);
    // Integer division truncates rather than rounding down.
    if base2_exp < 0 {
        base10_exp -= 1;
    }

    let mut adjust: i32 = if rounding_precision == 0 { 6 } else { 14 };
    let converted = Decimal128 { w: converted };
    let q = Decimal128 {
        w: [1, ((base10_exp - adjust + DECIMAL_EXPONENT_BIAS_128) as u64) << DECIMAL_COEFF_HIGH_BITS],
    };
    let mut quantized = Decimal128 { w: [0, 0] };
    bid128_quantize(&mut quantized, &converted, &q, &mut flags);
    (*this).w = quantized.w;

    if (*this).w[0] > largest_coeff {
        // The exponent guess was one out; quantize once more.
        adjust -= 1;
        let q1 = Decimal128 {
            w: [
                1,
                ((base10_exp as i64 - adjust as i64 + DECIMAL_EXPONENT_BIAS_128 as i64) as u64)
                    << DECIMAL_COEFF_HIGH_BITS,
            ],
        };
        bid128_quantize(&mut quantized, &converted, &q1, &mut flags);
        (*this).w = quantized.w;
    }
}

unsafe fn ctor_from_bid128_exp_sign(this: *mut Decimal128, coefficient: Decimal128, exponent: c_int, sign: bool) {
    let sign_x = if sign { MASK_SIGN } else { 0 };
    (*this).w = coefficient.w;
    let tmp = (exponent + DECIMAL_EXPONENT_BIAS_128) as u64;
    (*this).w[1] |= sign_x | (tmp << DECIMAL_COEFF_HIGH_BITS);
}

unsafe fn compare_body(this: *const Decimal128, rhs: *const Decimal128) -> c_int {
    let mut flags: c_uint = 0;
    let mut ret: c_int = 0;
    bid128_quiet_less(&mut ret, this, rhs, &mut flags);
    if ret != 0 {
        return -1;
    }
    bid128_quiet_greater(&mut ret, this, rhs, &mut flags);
    if ret != 0 {
        return 1;
    }

    // Either equal, or one or both is NaN.
    let l_nan = is_nan_val(&*this);
    let r_nan = is_nan_val(&*rhs);
    if !l_nan && !r_nan {
        return 0;
    }
    if l_nan && r_nan {
        // NaN ordering has to be stable.
        if (*this).w[1] == (*rhs).w[1] {
            return if (*this).w[0] == (*rhs).w[0] {
                0
            } else if (*this).w[0] < (*rhs).w[0] {
                -1
            } else {
                1
            };
        }
        return if (*this).w[1] < (*rhs).w[1] { -1 } else { 1 };
    }
    // NaN sorts before non-NaN.
    if l_nan {
        -1
    } else {
        1
    }
}

unsafe fn to_string_body(sret: *mut StdString, this: *const Decimal128) {
    if is_null_val(&*this) {
        // `return "NULL";` -- the const char* constructor, not an append.
        string_from_cstr(sret, b"NULL\0".as_ptr() as *const c_char);
        return;
    }

    let mut coefficient = Decimal128 { w: [0, 0] };
    let mut exponen: c_int = 0;
    let mut sign = false;
    unpack_body(this, &mut coefficient, &mut exponen, &mut sign);

    if coefficient.w[1] != 0 {
        // bid128_to_string writes a NUL-terminated string into a 64-byte buffer, which
        // the C then hands to the std::string(const char*) constructor.
        let mut buffer = [0u8; 64];
        let mut flags: c_uint = 0;
        bid128_to_string(buffer.as_mut_ptr() as *mut c_char, this, &mut flags);
        string_from_cstr(sret, buffer.as_ptr() as *const c_char);
        return;
    }

    // The significand is in w[0] only, which gets a nicer printout.
    *sret = StdString::EMPTY;
    if sign {
        string_append(sret, b"-\0".as_ptr() as *const c_char, 1);
    }

    if ((*this).w[1] & 0x7800_0000_0000_0000) == 0x7800_0000_0000_0000 {
        if ((*this).w[1] & NULL_HI) == NULL_HI {
            string_append(sret, b"NaN\0".as_ptr() as *const c_char, 3);
        } else {
            string_append(sret, b"Inf\0".as_ptr() as *const c_char, 3);
        }
        return;
    }

    let mut digits = to_string_uint(coefficient.w[0]);
    let (dptr, dlen) = digits.parts();
    let mut digits_before = dlen;
    while digits_before > 1 && exponen != 0 {
        digits_before -= 1;
        exponen += 1;
    }
    string_append(sret, dptr, digits_before);
    if digits_before < dlen {
        string_push_back(sret, b'.' as c_char);
        string_append(sret, dptr.add(digits_before), dlen - digits_before);
    }
    digits.destroy();

    if exponen != 0 {
        string_push_back(sret, b'E' as c_char);
        let mut e = to_string_int(exponen);
        let (eptr, elen) = e.parts();
        string_append(sret, eptr, elen);
        e.destroy();
    }
}

unsafe fn unpack_body(this: *const Decimal128, coefficient: *mut Decimal128, exponent: *mut c_int, sign: *mut bool) {
    *sign = ((*this).w[1] & MASK_SIGN) != 0;
    // The C reads the exponent into a *signed* 64-bit and shifts right; the field never
    // reaches bit 63, so this is an ordinary logical shift.
    let exp = ((*this).w[1] & MASK_EXP) as i64 >> DECIMAL_COEFF_HIGH_BITS;
    *exponent = exp as c_int - DECIMAL_EXPONENT_BIAS_128;
    (*coefficient).w[0] = (*this).w[0];
    (*coefficient).w[1] = coefficient_high(&*this);
}

// ---------------------------------------------------------------------------
// Exported surface. `C1` (complete) and `C2` (base) both forward to one body.
// ---------------------------------------------------------------------------

macro_rules! ctor_pair {
    ($c1:ident, $c2:ident, ($($arg:ident : $ty:ty),*), $body:expr) => {
        #[no_mangle]
        pub unsafe extern "C" fn $c1(this: *mut Decimal128 $(, $arg: $ty)*) {
            let f: unsafe fn(*mut Decimal128 $(, $ty)*) = $body;
            f(this $(, $arg)*)
        }
        #[no_mangle]
        pub unsafe extern "C" fn $c2(this: *mut Decimal128 $(, $arg: $ty)*) {
            let f: unsafe fn(*mut Decimal128 $(, $ty)*) = $body;
            f(this $(, $arg)*)
        }
    };
}

unsafe fn ctor_default(this: *mut Decimal128) {
    ctor_from_i64(this, 0)
}
unsafe fn ctor_from_int(this: *mut Decimal128, val: c_int) {
    ctor_from_i64(this, val as i64)
}
unsafe fn ctor_from_u64(this: *mut Decimal128, val: u64) {
    bid128_from_uint64(this, &val)
}
unsafe fn ctor_from_bid32(this: *mut Decimal128, u: u32) {
    if u == DECIMAL_NULL_32 {
        (*this).w = [NULL_LO, NULL_HI];
        return;
    }
    let mut flags: c_uint = 0;
    bid32_to_bid128(this, &u, &mut flags);
}
unsafe fn ctor_from_bid64(this: *mut Decimal128, w: u64) {
    if w == DECIMAL_NULL_64 {
        (*this).w = [NULL_LO, NULL_HI];
        return;
    }
    let mut flags: c_uint = 0;
    bid64_to_bid128(this, &w, &mut flags);
}
unsafe fn ctor_from_string_data(this: *mut Decimal128, init: StringData) {
    let mut flags: c_uint = 0;
    bid128_from_string(this, init.data as *mut c_char, &mut flags);
}
unsafe fn ctor_from_null(this: *mut Decimal128, _null: u8) {
    (*this).w = [NULL_LO, NULL_HI];
}
unsafe fn ctor_from_double_c(this: *mut Decimal128, val: f64, rounding: c_int) {
    ctor_from_double(this, val, rounding)
}

ctor_pair!(_ZN5realm10Decimal128C1Ev, _ZN5realm10Decimal128C2Ev, (), ctor_default);
ctor_pair!(_ZN5realm10Decimal128C1Ei, _ZN5realm10Decimal128C2Ei, (val: c_int), ctor_from_int);
ctor_pair!(_ZN5realm10Decimal128C1Ex, _ZN5realm10Decimal128C2Ex, (val: i64), ctor_from_i64);
ctor_pair!(_ZN5realm10Decimal128C1Ey, _ZN5realm10Decimal128C2Ey, (val: u64), ctor_from_u64);
ctor_pair!(_ZN5realm10Decimal128C1ENS0_5Bid32E, _ZN5realm10Decimal128C2ENS0_5Bid32E, (u: u32), ctor_from_bid32);
ctor_pair!(_ZN5realm10Decimal128C1ENS0_5Bid64E, _ZN5realm10Decimal128C2ENS0_5Bid64E, (w: u64), ctor_from_bid64);
ctor_pair!(_ZN5realm10Decimal128C1ENS_10StringDataE, _ZN5realm10Decimal128C2ENS_10StringDataE, (init: StringData), ctor_from_string_data);
ctor_pair!(_ZN5realm10Decimal128C1ENS_4nullE, _ZN5realm10Decimal128C2ENS_4nullE, (n: u8), ctor_from_null);
ctor_pair!(_ZN5realm10Decimal128C1EdNS0_7RoundToE, _ZN5realm10Decimal128C2EdNS0_7RoundToE, (val: f64, rounding: c_int), ctor_from_double_c);
ctor_pair!(_ZN5realm10Decimal128C1ENS0_6Bid128Eib, _ZN5realm10Decimal128C2ENS0_6Bid128Eib, (coefficient: Decimal128, exponent: c_int, sign: bool), ctor_from_bid128_exp_sign);

#[no_mangle]
pub unsafe extern "C" fn _ZN5realm10Decimal1283nanEPKc(init: *const c_char) -> Decimal128 {
    let mut val = Decimal128 { w: [0, 0] };
    val.w[0] = strtol(init, core::ptr::null_mut(), 10) as u64;
    val.w[1] = NULL_HI;
    // The C returns Decimal128(val), i.e. the explicit Bid128 constructor, which is the
    // inline one in the header: it assigns m_value and nothing else.
    val
}

#[no_mangle]
pub unsafe extern "C" fn _ZN5realm10Decimal12812is_valid_strENS_10StringDataE(str: StringData) -> bool {
    let mut flags: c_uint = 0;
    let mut tmp = Decimal128 { w: [0, 0] };
    bid128_from_string(&mut tmp, str.data as *mut c_char, &mut flags);
    (tmp.w[1] & NULL_HI) != NULL_HI
}

#[no_mangle]
pub unsafe extern "C" fn _ZNK5realm10Decimal1287is_nullEv(this: *const Decimal128) -> bool {
    is_null_val(&*this)
}

#[no_mangle]
pub unsafe extern "C" fn _ZNK5realm10Decimal1286is_nanEv(this: *const Decimal128) -> bool {
    is_nan_val(&*this)
}

#[no_mangle]
pub unsafe extern "C" fn _ZNK5realm10Decimal1286to_intERx(this: *const Decimal128, i: *mut i64) -> bool {
    let mut res: i64 = 0;
    let mut flags: c_uint = 0;
    bid128_to_int64_int(&mut res, this, &mut flags);
    if flags == 0 {
        *i = res;
        return true;
    }
    false
}

#[no_mangle]
pub unsafe extern "C" fn _ZNK5realm10Decimal128eqERKS0_(this: *const Decimal128, rhs: *const Decimal128) -> bool {
    if is_null_val(&*this) && is_null_val(&*rhs) {
        return true;
    }
    let mut flags: c_uint = 0;
    let mut ret: c_int = 0;
    bid128_quiet_equal(&mut ret, this, rhs, &mut flags);
    if ret != 0 {
        return true;
    }
    if is_nan_val(&*this) && is_nan_val(&*rhs) {
        return (*this).w[1] == (*rhs).w[1] && (*this).w[0] == (*rhs).w[0];
    }
    false
}

#[no_mangle]
pub unsafe extern "C" fn _ZNK5realm10Decimal128neERKS0_(this: *const Decimal128, rhs: *const Decimal128) -> bool {
    !_ZNK5realm10Decimal128eqERKS0_(this, rhs)
}

#[no_mangle]
pub unsafe extern "C" fn _ZNK5realm10Decimal1287compareERKS0_(this: *const Decimal128, rhs: *const Decimal128) -> c_int {
    compare_body(this, rhs)
}

#[no_mangle]
pub unsafe extern "C" fn _ZNK5realm10Decimal128ltERKS0_(this: *const Decimal128, rhs: *const Decimal128) -> bool {
    compare_body(this, rhs) < 0
}
#[no_mangle]
pub unsafe extern "C" fn _ZNK5realm10Decimal128gtERKS0_(this: *const Decimal128, rhs: *const Decimal128) -> bool {
    compare_body(this, rhs) > 0
}
#[no_mangle]
pub unsafe extern "C" fn _ZNK5realm10Decimal128leERKS0_(this: *const Decimal128, rhs: *const Decimal128) -> bool {
    compare_body(this, rhs) <= 0
}
#[no_mangle]
pub unsafe extern "C" fn _ZNK5realm10Decimal128geERKS0_(this: *const Decimal128, rhs: *const Decimal128) -> bool {
    compare_body(this, rhs) >= 0
}

#[inline]
unsafe fn do_multiply(x: Decimal128, mul: Decimal128) -> Decimal128 {
    let mut flags: c_uint = 0;
    let mut res = Decimal128 { w: [0, 0] };
    bid128_mul(&mut res, &x, &mul, &mut flags);
    res
}

#[inline]
unsafe fn do_divide(x: Decimal128, div: Decimal128) -> Decimal128 {
    let mut flags: c_uint = 0;
    let mut res = Decimal128 { w: [0, 0] };
    bid128_div(&mut res, &x, &div, &mut flags);
    res
}

#[no_mangle]
pub unsafe extern "C" fn _ZNK5realm10Decimal128mlEx(this: *const Decimal128, mul: i64) -> Decimal128 {
    let mut y = Decimal128 { w: [0, 0] };
    ctor_from_i64(&mut y, mul);
    do_multiply(*this, y)
}
#[no_mangle]
pub unsafe extern "C" fn _ZNK5realm10Decimal128mlEm(this: *const Decimal128, mul: usize) -> Decimal128 {
    let mut y = Decimal128 { w: [0, 0] };
    ctor_from_u64(&mut y, mul as u64);
    do_multiply(*this, y)
}
#[no_mangle]
pub unsafe extern "C" fn _ZNK5realm10Decimal128mlEi(this: *const Decimal128, mul: c_int) -> Decimal128 {
    let mut y = Decimal128 { w: [0, 0] };
    ctor_from_i64(&mut y, mul as i64);
    do_multiply(*this, y)
}
#[no_mangle]
pub unsafe extern "C" fn _ZNK5realm10Decimal128mlES0_(this: *const Decimal128, mul: Decimal128) -> Decimal128 {
    do_multiply(*this, mul)
}

#[no_mangle]
pub unsafe extern "C" fn _ZNK5realm10Decimal128dvEx(this: *const Decimal128, div: i64) -> Decimal128 {
    let mut y = Decimal128 { w: [0, 0] };
    ctor_from_i64(&mut y, div);
    do_divide(*this, y)
}
#[no_mangle]
pub unsafe extern "C" fn _ZNK5realm10Decimal128dvEm(this: *const Decimal128, div: usize) -> Decimal128 {
    let mut y = Decimal128 { w: [0, 0] };
    ctor_from_u64(&mut y, div as u64);
    do_divide(*this, y)
}
#[no_mangle]
pub unsafe extern "C" fn _ZNK5realm10Decimal128dvEi(this: *const Decimal128, div: c_int) -> Decimal128 {
    let mut y = Decimal128 { w: [0, 0] };
    ctor_from_i64(&mut y, div as i64);
    do_divide(*this, y)
}
#[no_mangle]
pub unsafe extern "C" fn _ZNK5realm10Decimal128dvES0_(this: *const Decimal128, div: Decimal128) -> Decimal128 {
    do_divide(*this, div)
}

#[no_mangle]
pub unsafe extern "C" fn _ZN5realm10Decimal128pLES0_(this: *mut Decimal128, rhs: Decimal128) -> *mut Decimal128 {
    let mut flags: c_uint = 0;
    let x = *this;
    let mut res = Decimal128 { w: [0, 0] };
    bid128_add(&mut res, &x, &rhs, &mut flags);
    (*this).w = res.w;
    this
}

#[no_mangle]
pub unsafe extern "C" fn _ZN5realm10Decimal128mIES0_(this: *mut Decimal128, rhs: Decimal128) -> *mut Decimal128 {
    let mut flags: c_uint = 0;
    let x = *this;
    let mut res = Decimal128 { w: [0, 0] };
    bid128_sub(&mut res, &x, &rhs, &mut flags);
    (*this).w = res.w;
    this
}

#[no_mangle]
pub unsafe extern "C" fn _ZNK5realm10Decimal1289to_stringEv(sret: *mut StdString, this: *const Decimal128) {
    to_string_body(sret, this)
}

#[no_mangle]
pub unsafe extern "C" fn _ZNK5realm10Decimal1288to_bid32Ev(this: *const Decimal128) -> OptBid32 {
    if is_null_val(&*this) {
        return OptBid32 { value: DECIMAL_NULL_32, engaged: true };
    }
    let mut flags: c_uint = 0;
    let mut buffer: u32 = 0;
    bid128_to_bid32(&mut buffer, this, &mut flags);
    if flags & !BID_INEXACT_EXCEPTION != 0 {
        return OptBid32 { value: 0, engaged: false };
    }
    OptBid32 { value: buffer, engaged: true }
}

#[no_mangle]
pub unsafe extern "C" fn _ZNK5realm10Decimal1288to_bid64Ev(this: *const Decimal128) -> OptBid64 {
    if is_null_val(&*this) {
        return OptBid64 { value: DECIMAL_NULL_64, engaged: true };
    }
    let mut flags: c_uint = 0;
    let mut buffer: u64 = 0;
    bid128_to_bid64(&mut buffer, this, &mut flags);
    if flags & !BID_INEXACT_EXCEPTION != 0 {
        return OptBid64 { value: 0, engaged: false };
    }
    OptBid64 { value: buffer, engaged: true }
}

#[no_mangle]
pub unsafe extern "C" fn _ZNK5realm10Decimal1286unpackERNS0_6Bid128ERiRb(
    this: *const Decimal128,
    coefficient: *mut Decimal128,
    exponent: *mut c_int,
    sign: *mut bool,
) {
    unpack_body(this, coefficient, exponent, sign)
}

/// `realm::operator==(Decimal128::Bid32, Decimal128::Bid32)` — a free function, so the
/// two `Bid32`s arrive as two plain `uint32_t` in `edi`/`esi`.
#[no_mangle]
pub unsafe extern "C" fn _ZN5realmeqENS_10Decimal1285Bid32ES1_(x: u32, y: u32) -> bool {
    const DECIMAL_COEFF_BITS_32: u32 = 23;
    const DECIMAL_EXP_BITS_32: u32 = 8;
    const MASK_COEFF_32: u32 = (1 << DECIMAL_COEFF_BITS_32) - 1;
    const MASK_EXP_32: u32 = ((1 << DECIMAL_EXP_BITS_32) - 1) << DECIMAL_COEFF_BITS_32;
    const MASK_SIGN_32: u32 = 1 << (DECIMAL_COEFF_BITS_32 + DECIMAL_EXP_BITS_32);

    if x == y {
        return true;
    }

    let mut sig_x = x & MASK_COEFF_32;
    let mut sig_y = y & MASK_COEFF_32;
    let x_is_zero = sig_x == 0;
    let y_is_zero = sig_y == 0;

    if x_is_zero && y_is_zero {
        return true;
    } else if (x_is_zero && !y_is_zero) || (!x_is_zero && y_is_zero) {
        return false;
    }

    if (x ^ y) & MASK_SIGN_32 != 0 {
        return false;
    }

    let mut exp_x = ((x & MASK_EXP_32) >> DECIMAL_COEFF_BITS_32) as i32;
    let mut exp_y = ((y & MASK_EXP_32) >> DECIMAL_COEFF_BITS_32) as i32;

    // Make exp_y the bigger one.
    if exp_x > exp_y {
        core::mem::swap(&mut exp_x, &mut exp_y);
        core::mem::swap(&mut sig_x, &mut sig_y);
    }
    if exp_y - exp_x > 6 {
        return false;
    }
    for _ in 0..(exp_y - exp_x) {
        sig_y = sig_y.wrapping_mul(10);
        if sig_y > 9_999_999 {
            return false;
        }
    }
    sig_y == sig_x
}
