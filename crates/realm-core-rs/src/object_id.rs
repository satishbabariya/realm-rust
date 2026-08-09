//! Port of `upstream/src/realm/object_id.cpp`.
//!
//! **Byte-visible.** `ObjectId` is a 12-byte value written into `.realm` files, and
//! upstream guards it with
//! `static_assert(sizeof(ObjectId) == 12, "changing the size of an ObjectId is a file
//! format breaking change")`. The byte order inside it is deliberate, not incidental —
//! see the two format notes below.
//!
//! **Byte-visible but untraced**: the trace schema is scalar-only, so no trace stores an
//! `ObjectId`. `make verify` proves the link and that nothing else regressed; the
//! evidence is `migration/checks/run_object_id_differential.sh`.
//!
//! # Format decisions (these reach the file)
//!
//! 1. **Seconds and sequence are stored big-endian**, explicitly so that `memcmp`
//!    orders `ObjectId`s the same way their timestamps order. Bytes 0..4 are the
//!    seconds, bytes 9..12 the low 24 bits of the counter. A little-endian port would
//!    round-trip perfectly through its own code and sort wrongly against every other
//!    binding.
//! 2. **`machine_id` and `process_id` are stored little-endian**, and by accident of
//!    how: the C++ is `memcpy(m_bytes.data() + 4, &machine_id, 3)` and
//!    `memcpy(m_bytes.data() + 7, &process_id, 2)` — the *low* 3 and 2 bytes of a
//!    native `int`. On x86-64 that is the low-order bytes; the same source on a
//!    big-endian host would store the high-order ones. Mirrored as an explicit
//!    little-endian truncation rather than a `memcpy`, with the divergence noted.
//!
//! So one struct stores two fields big-endian and two little-endian. That is worth
//! saying out loud, because "fix the inconsistency" is a file-format break.
//!
//! # ABI, read off the oracle's disassembly
//!
//! | Function | convention |
//! |---|---|
//! | `to_bytes()` | `this` in `rdi`, 12 bytes returned in `rax:edx` |
//! | `get_timestamp()` | `this` in `rdi`, 16 bytes in `rax:rdx`; the compiler emits `bswapl` |
//! | `to_string()` | **`sret` in `rdi`**, `this` in `rsi` |
//! | `gen()` | static, no `this`, `ObjectId` in `rax:edx` |
//!
//! `ObjectId` (12 B), `Timestamp` (16 B) and `std::array<unsigned char, 12>` are all
//! trivially copyable, so they return in registers. `std::string` is not, so it does
//! not — the distinction that broke `status.cpp`.
//!
//! # The static initialiser
//!
//! `object_id.cpp` has a namespace-scope `g_gen_state` whose constructor draws three
//! `std::random_device` values, so the object file carries a
//! `__GLOBAL__sub_I_object_id.cpp` that runs before `main`. Rust has no equivalent, so
//! this port initialises the same state **lazily** on first use.
//!
//! That is a real behavioural difference in *timing*, and it is unobservable in
//! *values*: every consumer of that state (`gen()`'s machine/process id, and the
//! sequence counter's starting point) is random in both stacks, so no two runs of
//! either stack agree anyway. Recorded rather than hidden — if a future trace ever
//! stores a generated `ObjectId`, `make determinism-check` will fail on the **oracle**
//! first, and that is a property of upstream, not of this port.

use core::ffi::{c_char, c_void};
use core::sync::atomic::{AtomicU32, Ordering};

/// `realm::ObjectId` — 12 bytes, alignment 1.
#[repr(C)]
pub struct ObjectId {
    m_bytes: [u8; 12],
}

/// `std::array<unsigned char, 12>`, the type `to_bytes()` returns.
#[repr(C)]
pub struct ObjectIdBytes {
    elems: [u8; 12],
}

/// `realm::StringData` — `{const char*, size_t}`.
#[repr(C)]
pub struct StringData {
    data: *const c_char,
    size: usize,
}

/// `realm::Timestamp`.
#[repr(C)]
pub struct Timestamp {
    m_seconds: i64,
    m_nanoseconds: i32,
    m_is_null: bool,
}

/// `std::string`, as 24 raw bytes. Only ever *built* here, never read.
#[repr(C)]
pub struct StdString {
    rep: [u8; 24],
}

const _: () = {
    // The file-format invariant upstream static_asserts.
    assert!(core::mem::size_of::<ObjectId>() == 12);
    assert!(core::mem::align_of::<ObjectId>() == 1);
    assert!(core::mem::size_of::<ObjectIdBytes>() == 12);
    assert!(core::mem::size_of::<StringData>() == 16);
    assert!(core::mem::size_of::<Timestamp>() == 16);
    assert!(core::mem::offset_of!(Timestamp, m_nanoseconds) == 8);
    assert!(core::mem::offset_of!(Timestamp, m_is_null) == 12);
    assert!(core::mem::size_of::<StdString>() == 24);
};

extern "C-unwind" {
    #[link_name = "_Znwm"]
    fn cxx_operator_new(size: usize) -> *mut u8;
}

extern "C" {
    /// `realm::murmur2_or_cityhash(const unsigned char*, size_t)` — already ported;
    /// this binds to the Rust definition through the C++ mangled name so there is one
    /// implementation of that behaviour, not two.
    #[link_name = "_ZN5realm19murmur2_or_cityhashEPKhm"]
    fn murmur2_or_cityhash(data: *const u8, len: usize) -> usize;

    fn time(t: *mut c_void) -> i64;
}

// ---------------------------------------------------------------------------
// g_gen_state — see the module comment for why this is lazy rather than pre-main.
// ---------------------------------------------------------------------------
struct GeneratorState {
    machine_id: i32,
    process_id: i32,
    seq: AtomicU32,
}

static GEN_STATE: std::sync::OnceLock<GeneratorState> = std::sync::OnceLock::new();

fn gen_state() -> &'static GeneratorState {
    GEN_STATE.get_or_init(|| {
        // `std::random_device{}()` three times, 4 bytes each. The C++ static_asserts
        // that width. Any nondeterministic 32-bit source satisfies the same contract;
        // the values are never compared against the oracle's because both are random.
        let mut buf = [0u8; 12];
        getentropy_or_fallback(&mut buf);
        GeneratorState {
            machine_id: i32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]),
            process_id: i32::from_ne_bytes([buf[4], buf[5], buf[6], buf[7]]),
            seq: AtomicU32::new(u32::from_ne_bytes([buf[8], buf[9], buf[10], buf[11]])),
        }
    })
}

fn getentropy_or_fallback(out: &mut [u8; 12]) {
    extern "C" {
        fn getentropy(buf: *mut c_void, len: usize) -> i32;
    }
    let ok = unsafe { getentropy(out.as_mut_ptr() as *mut c_void, out.len()) == 0 };
    if !ok {
        // getentropy cannot fail for a 12-byte request on Darwin, but a silent zero
        // state would be a collision generator, so fall back to something varying.
        let t = unsafe { time(core::ptr::null_mut()) } as u64;
        let mix = t.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        out[..8].copy_from_slice(&mix.to_ne_bytes());
        out[8..].copy_from_slice(&(mix as u32).to_ne_bytes());
    }
}

/// `std::isxdigit` in the C locale. The C++ calls the locale-aware form (the object
/// imports `__DefaultRuneLocale`), but realm never installs a locale, and no locale in
/// practice adds hex digits outside ASCII.
#[inline]
fn is_xdigit(c: u8) -> bool {
    c.is_ascii_hexdigit()
}

/// Value of one hex digit, mirroring what `strtol(base 16)` accepts.
#[inline]
fn hex_val(c: u8) -> i64 {
    match c {
        b'0'..=b'9' => (c - b'0') as i64,
        b'a'..=b'f' => (c - b'a') as i64 + 10,
        b'A'..=b'F' => (c - b'A') as i64 + 10,
        _ => 0,
    }
}

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

// ===========================================================================
// Exported symbols.
// ===========================================================================

/// `realm::ObjectId::is_valid_str(StringData)`
#[export_name = "_ZN5realm8ObjectId12is_valid_strENS_10StringDataE"]
pub unsafe extern "C" fn object_id_is_valid_str(str_: StringData) -> bool {
    if str_.size != 24 {
        return false;
    }
    let bytes = core::slice::from_raw_parts(str_.data as *const u8, 24);
    bytes.iter().all(|&c| is_xdigit(c))
}

/// Shared body of the `C1`/`C2` variants of `ObjectId::ObjectId(StringData)`.
///
/// `REALM_ASSERT(is_valid_str(init))` is a no-op in this build, so an ill-formed input
/// is parsed rather than rejected — mirrored, including `strtol`'s treatment of a
/// non-hex byte as 0.
#[inline]
unsafe fn construct_from_str(this: *mut ObjectId, init: StringData) {
    let src = init.data as *const u8;
    let mut j = 0usize;
    for i in 0..12usize {
        let hi = *src.add(j);
        let lo = *src.add(j + 1);
        j += 2;
        (*this).m_bytes[i] = ((hex_val(hi) << 4) | hex_val(lo)) as u8;
    }
}

#[export_name = "_ZN5realm8ObjectIdC1ENS_10StringDataE"]
pub unsafe extern "C" fn object_id_ctor_str_c1(this: *mut ObjectId, init: StringData) {
    construct_from_str(this, init);
}

#[export_name = "_ZN5realm8ObjectIdC2ENS_10StringDataE"]
pub unsafe extern "C" fn object_id_ctor_str_c2(this: *mut ObjectId, init: StringData) {
    construct_from_str(this, init);
}

#[inline]
unsafe fn construct_from_bytes(this: *mut ObjectId, init: *const ObjectIdBytes) {
    (*this).m_bytes = (*init).elems;
}

#[export_name = "_ZN5realm8ObjectIdC1ERKNSt3__15arrayIhLm12EEE"]
pub unsafe extern "C" fn object_id_ctor_bytes_c1(this: *mut ObjectId, init: *const ObjectIdBytes) {
    construct_from_bytes(this, init);
}

#[export_name = "_ZN5realm8ObjectIdC2ERKNSt3__15arrayIhLm12EEE"]
pub unsafe extern "C" fn object_id_ctor_bytes_c2(this: *mut ObjectId, init: *const ObjectIdBytes) {
    construct_from_bytes(this, init);
}

/// Shared body of `ObjectId::ObjectId(Timestamp, int machine_id, int process_id)`.
///
/// Two byte orders in one function; see the module comment.
#[inline]
unsafe fn construct_from_timestamp(this: *mut ObjectId, d: Timestamp, machine_id: i32, process_id: i32) {
    let sec = d.m_seconds as u32; // uint32_t(d.get_seconds())

    // Big-endian, deliberately: so memcmp orders by time.
    (*this).m_bytes[0] = (sec >> 24) as u8;
    (*this).m_bytes[1] = ((sec >> 16) & 0xff) as u8;
    (*this).m_bytes[2] = ((sec >> 8) & 0xff) as u8;
    (*this).m_bytes[3] = (sec & 0xff) as u8;

    // memcpy of the low 3 and 2 bytes of a native int -- little-endian on this target.
    let m = machine_id.to_ne_bytes();
    (*this).m_bytes[4] = m[0];
    (*this).m_bytes[5] = m[1];
    (*this).m_bytes[6] = m[2];
    let p = process_id.to_ne_bytes();
    (*this).m_bytes[7] = p[0];
    (*this).m_bytes[8] = p[1];

    let r = gen_state().seq.fetch_add(1, Ordering::Relaxed);

    // Big-endian again, so that ids made in the same second still sort by creation.
    (*this).m_bytes[9] = ((r >> 16) & 0xff) as u8;
    (*this).m_bytes[10] = ((r >> 8) & 0xff) as u8;
    (*this).m_bytes[11] = (r & 0xff) as u8;
}

#[export_name = "_ZN5realm8ObjectIdC1ENS_9TimestampEii"]
pub unsafe extern "C" fn object_id_ctor_ts_c1(
    this: *mut ObjectId,
    d: Timestamp,
    machine_id: i32,
    process_id: i32,
) {
    construct_from_timestamp(this, d, machine_id, process_id);
}

#[export_name = "_ZN5realm8ObjectIdC2ENS_9TimestampEii"]
pub unsafe extern "C" fn object_id_ctor_ts_c2(
    this: *mut ObjectId,
    d: Timestamp,
    machine_id: i32,
    process_id: i32,
) {
    construct_from_timestamp(this, d, machine_id, process_id);
}

/// `realm::ObjectId::gen()` — static, returns `ObjectId` in `rax:edx`.
#[export_name = "_ZN5realm8ObjectId3genEv"]
pub unsafe extern "C" fn object_id_gen() -> ObjectId {
    let st = gen_state();
    let mut out = ObjectId { m_bytes: [0u8; 12] };
    let now = time(core::ptr::null_mut());
    construct_from_timestamp(
        &mut out,
        Timestamp {
            m_seconds: now,
            m_nanoseconds: 0,
            m_is_null: false,
        },
        st.machine_id,
        st.process_id,
    );
    out
}

/// `realm::ObjectId::get_timestamp() const`
///
/// `Timestamp(sec, 0)` — nanoseconds 0 and `m_is_null` false, which is why the oracle
/// zeroes the whole second return register.
#[export_name = "_ZNK5realm8ObjectId13get_timestampEv"]
pub unsafe extern "C" fn object_id_get_timestamp(this: *const ObjectId) -> Timestamp {
    let b = &(*this).m_bytes;
    let sec = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    Timestamp {
        m_seconds: sec as i64,
        m_nanoseconds: 0,
        m_is_null: false,
    }
}

/// `realm::ObjectId::to_string() const` — **`sret` in `rdi`, `this` in `rsi`**.
///
/// Always produces exactly 24 characters (`2 * sizeof(ObjectIdBytes)`), which is past
/// libc++'s 22-byte short-string limit, so the result is always heap-allocated. The
/// representation is reproduced from measurement rather than from libc++'s
/// `__recommend`, because only this one length is ever produced:
///
/// ```text
/// n=22 -> short, capacity 22
/// n=23 -> long,  capacity 25   (stored __cap_ 13)
/// n=24 -> long,  capacity 31   (stored __cap_ 16)   <- the case here
/// ```
///
/// `capacity()` is `__cap_ * 2 - 1`, so `__cap_` is 16 and the allocation is 32 bytes
/// (24 characters, a NUL, and slack). The differential compares `size()`, `capacity()`
/// and the characters against the oracle, so a wrong capacity fails there.
#[export_name = "_ZNK5realm8ObjectId9to_stringEv"]
pub unsafe extern "C-unwind" fn object_id_to_string(
    out: *mut StdString,
    this: *const ObjectId,
) -> *mut StdString {
    const N: usize = 24;
    const CAP_FIELD: usize = 16; // capacity() == 31
    const ALLOC: usize = 32;

    let buf = cxx_operator_new(ALLOC);
    let b = &(*this).m_bytes;
    for i in 0..12usize {
        let c = b[i];
        *buf.add(2 * i) = HEX_DIGITS[(c >> 4) as usize];
        *buf.add(2 * i + 1) = HEX_DIGITS[(c & 0xf) as usize];
    }
    *buf.add(N) = 0;
    // Slack beyond the NUL is left as operator new returned it, matching C++.

    let rep = &mut (*out).rep;
    rep[0..8].copy_from_slice(&(((CAP_FIELD) << 1) | 1).to_ne_bytes());
    rep[8..16].copy_from_slice(&N.to_ne_bytes());
    rep[16..24].copy_from_slice(&(buf as usize).to_ne_bytes());
    out
}

/// `realm::ObjectId::to_bytes() const` — 12 bytes in `rax:edx`.
#[export_name = "_ZNK5realm8ObjectId8to_bytesEv"]
pub unsafe extern "C" fn object_id_to_bytes(this: *const ObjectId) -> ObjectIdBytes {
    ObjectIdBytes {
        elems: (*this).m_bytes,
    }
}

/// `realm::ObjectId::hash() const`
#[export_name = "_ZNK5realm8ObjectId4hashEv"]
pub unsafe extern "C" fn object_id_hash(this: *const ObjectId) -> usize {
    murmur2_or_cityhash((*this).m_bytes.as_ptr(), 12)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sd(s: &str) -> StringData {
        StringData {
            data: s.as_ptr() as *const c_char,
            size: s.len(),
        }
    }

    #[test]
    fn is_valid_str_requires_exactly_24_hex_digits() {
        unsafe {
            assert!(object_id_is_valid_str(sd("000102030405060708090a0b")));
            assert!(object_id_is_valid_str(sd("ABCDEFabcdef0123456789AB")));
            assert!(!object_id_is_valid_str(sd("000102030405060708090a0")));  // 23
            assert!(!object_id_is_valid_str(sd("000102030405060708090a0bc"))); // 25
            assert!(!object_id_is_valid_str(sd("000102030405060708090a0g"))); // non-hex
            assert!(!object_id_is_valid_str(sd("")));
        }
    }

    #[test]
    fn string_round_trip() {
        let hex = "0123456789abcdef01234567";
        let mut oid = ObjectId { m_bytes: [0; 12] };
        unsafe { construct_from_str(&mut oid, sd(hex)) };
        assert_eq!(
            oid.m_bytes,
            [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67]
        );
    }

    #[test]
    fn uppercase_hex_parses_the_same() {
        let mut lower = ObjectId { m_bytes: [0; 12] };
        let mut upper = ObjectId { m_bytes: [0; 12] };
        unsafe {
            construct_from_str(&mut lower, sd("0123456789abcdef01234567"));
            construct_from_str(&mut upper, sd("0123456789ABCDEF01234567"));
        }
        assert_eq!(lower.m_bytes, upper.m_bytes);
    }

    #[test]
    fn seconds_are_big_endian_so_memcmp_orders_by_time() {
        let mk = |secs: i64| {
            let mut o = ObjectId { m_bytes: [0; 12] };
            unsafe {
                construct_from_timestamp(
                    &mut o,
                    Timestamp {
                        m_seconds: secs,
                        m_nanoseconds: 0,
                        m_is_null: false,
                    },
                    0,
                    0,
                )
            };
            o
        };
        let a = mk(1000);
        let b = mk(2000);
        // Byte order, not numeric comparison: this is the property the layout exists for.
        assert!(a.m_bytes[..4] < b.m_bytes[..4]);
        assert_eq!(u32::from_be_bytes([a.m_bytes[0], a.m_bytes[1], a.m_bytes[2], a.m_bytes[3]]), 1000);
    }

    #[test]
    fn machine_and_process_id_are_stored_little_endian() {
        let mut o = ObjectId { m_bytes: [0; 12] };
        unsafe {
            construct_from_timestamp(
                &mut o,
                Timestamp {
                    m_seconds: 0,
                    m_nanoseconds: 0,
                    m_is_null: false,
                },
                0x00_11_22_33,
                0x00_00_44_55,
            )
        };
        // memcpy takes the LOW 3 and 2 bytes of the native int.
        assert_eq!(&o.m_bytes[4..7], &[0x33, 0x22, 0x11]);
        assert_eq!(&o.m_bytes[7..9], &[0x55, 0x44]);
    }

    #[test]
    fn get_timestamp_recovers_the_seconds_and_zeroes_the_rest() {
        let mut o = ObjectId { m_bytes: [0; 12] };
        unsafe {
            construct_from_timestamp(
                &mut o,
                Timestamp {
                    m_seconds: 0x1234_5678,
                    m_nanoseconds: 999,
                    m_is_null: true,
                },
                0,
                0,
            )
        };
        let ts = unsafe { object_id_get_timestamp(&o) };
        assert_eq!(ts.m_seconds, 0x1234_5678);
        assert_eq!(ts.m_nanoseconds, 0);
        assert!(!ts.m_is_null);
    }

    #[test]
    fn sequence_counter_increments_by_one_per_construction() {
        let mk = || {
            let mut o = ObjectId { m_bytes: [0; 12] };
            unsafe {
                construct_from_timestamp(
                    &mut o,
                    Timestamp {
                        m_seconds: 0,
                        m_nanoseconds: 0,
                        m_is_null: false,
                    },
                    0,
                    0,
                )
            };
            u32::from_be_bytes([0, o.m_bytes[9], o.m_bytes[10], o.m_bytes[11]])
        };
        let a = mk();
        let b = mk();
        // The absolute value is random per process; the delta is not.
        assert_eq!(b.wrapping_sub(a) & 0x00ff_ffff, 1);
    }
}
