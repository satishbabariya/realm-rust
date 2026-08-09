//! Port of `upstream/src/realm/array_timestamp.cpp`.
//!
//! **Byte-visible but untraced.** No trace stores a `Timestamp` — the schema is
//! int/string/double/bool — so `make verify` proves only that the link is intact and
//! nothing else regressed. The evidence is
//! `migration/checks/run_array_timestamp_differential.sh`.
//!
//! # A third multiple-inheritance shape
//!
//! `ArrayTimestamp : public ArrayPayload, public Array`, and `ArrayPayload` is the
//! **primary** base — so unlike the blob units, where `Array` sat at offset 0, the
//! `Array` subobject starts at **8**.
//!
//! ```text
//!   0 | (ArrayPayload vtable pointer)          &vtable[2]
//!   8 |   class realm::Array                   Node vptr here,     &vtable[11]
//!  64 |     (ArrayParent vtable pointer)                           &vtable[19]
//! 120 |   ArrayIntNull  m_seconds              120 bytes, three vptrs of its own
//! 240 |   ArrayInteger  m_nanoseconds          120 bytes, three vptrs of its own
//!       [sizeof = 360]
//! ```
//!
//! And the sub-arrays are themselves multiply inherited —
//! `ArrayIntNull : public Array, public ArrayPayload`, 120 bytes:
//!
//! ```text
//!   0 | (Node vtable pointer)          &vtable[2]
//!  56 | (ArrayParent vtable pointer)   &vtable[13]
//! 112 | (ArrayPayload vtable pointer)  &vtable[19]
//! ```
//!
//! So constructing an `ArrayTimestamp` writes **nine** vtable pointers across three
//! objects. Every offset above was measured from a real object, because
//! `-fdump-vtable-layouts` emits nothing for a TU that is not the key-function TU.
//!
//! # `ArrayIntNull` is an encoding, not infrastructure
//!
//! Eight of the methods this unit calls on `m_seconds` have no out-of-line definition:
//! `get`, `size`, `insert`, `set`, `set_null`, `is_null`, `erase`, `clear`. That is the
//! shape that nearly caused `array_blob` to be parked, so the bodies were read rather
//! than assumed. They are one-to-three-liners over an index-plus-one offset and a
//! sentinel stored in element 0:
//!
//! ```text
//! size()       Array::size() - 1
//! null_value() Array::get(0)
//! get(ndx)     v = Array::get(ndx+1); v == null_value() ? none : some(v)
//! set(ndx, v)  v ? (avoid_null_collision(*v), Array::set(ndx+1, *v))
//!                : Array::set(ndx+1, null_value())
//! ```
//!
//! The genuinely hard part — choosing a new sentinel when a stored value collides with
//! it, and rewriting the array — is `avoid_null_collision`, which **is** out-of-line
//! and bound here. Contrast `array_blobs_big`, parked the same day, where the missing
//! inline body was B+-tree leaf insertion with splitting. The test is the *content* of
//! the body, not its absence from the symbol table.
//!
//! All seventeen `ArrayIntNull::find_first<Cond>` instantiations are likewise defined
//! in `librealm.a` and bound rather than reimplemented.

use core::ffi::c_void;

use crate::array_blob::{array_get, Array};
use crate::array_unsigned::{Allocator, MemRef};

/// `realm::ArrayIntNull` / `realm::ArrayInteger` — `Array` plus an `ArrayPayload` base.
#[repr(C)]
pub struct ArrayIntNull {
    arr: Array,                  // 0   (112 bytes, Node vptr at 0, ArrayParent vptr at 56)
    vptr_payload: *const c_void, // 112
}

/// `realm::ArrayTimestamp`.
#[repr(C)]
pub struct ArrayTimestamp {
    vptr_payload: *const c_void, // 0   ArrayPayload is the PRIMARY base here
    base: Array,                 // 8
    m_seconds: ArrayIntNull,     // 120
    m_nanoseconds: ArrayIntNull, // 240 (an ArrayInteger; identical layout)
}

/// `realm::Timestamp`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Timestamp {
    m_seconds: i64,
    m_nanoseconds: i32,
    m_is_null: bool,
}

/// `std::optional<int64_t>` — value at 0, engaged flag at 8, as measured for `base64`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct OptI64 {
    value: i64,
    engaged: bool,
}

impl OptI64 {
    const NONE: Self = OptI64 {
        value: 0,
        engaged: false,
    };
    fn some(v: i64) -> Self {
        OptI64 {
            value: v,
            engaged: true,
        }
    }
}

const _: () = {
    assert!(core::mem::size_of::<Array>() == 112);
    assert!(core::mem::size_of::<ArrayIntNull>() == 120);
    assert!(core::mem::size_of::<ArrayTimestamp>() == 360);
    assert!(core::mem::offset_of!(ArrayTimestamp, base) == 8);
    assert!(core::mem::offset_of!(ArrayTimestamp, m_seconds) == 120);
    assert!(core::mem::offset_of!(ArrayTimestamp, m_nanoseconds) == 240);
    assert!(core::mem::offset_of!(ArrayIntNull, vptr_payload) == 112);
    assert!(core::mem::size_of::<Timestamp>() == 16);
    assert!(core::mem::size_of::<OptI64>() == 16);
};

/// Measured vtable slot indices. `-fdump-vtable-layouts` emits nothing for a TU that is
/// not the key-function TU, so these came off real objects; the differential re-checks
/// them by constructing one and comparing behaviour against the oracle.
mod vslots {
    /// `ArrayTimestamp`: ArrayPayload vptr at object offset 0, Node at 8, ArrayParent at 64.
    pub const TS_PAYLOAD: usize = 2;
    pub const TS_NODE: usize = 11;
    pub const TS_ARRAY_PARENT: usize = 19;
    /// `ArrayIntNull` / `ArrayInteger`: Node at 0, ArrayParent at 56, ArrayPayload at 112.
    pub const SUB_NODE: usize = 2;
    pub const SUB_ARRAY_PARENT: usize = 13;
    pub const SUB_PAYLOAD: usize = 19;
    /// `ArrayParent::get_child_ref`, measured for `array_unsigned`.
    pub const GET_CHILD_REF: usize = 2;
}

/// Byte offset of `ArrayTimestamp`'s `ArrayParent` subobject — what `this` becomes when
/// passed to `set_parent`.
const TS_ARRAY_PARENT_OFFSET: usize = 64;

const NOT_FOUND: usize = usize::MAX;
const TYPE_NORMAL: i32 = 0;
const TYPE_HAS_REFS: i32 = 2;
const WTYPE_BITS: i32 = 0;

extern "C" {
    #[link_name = "_ZTVN5realm14ArrayTimestampE"]
    static TS_VTABLE: [*const c_void; 0];
    #[link_name = "_ZTVN5realm12ArrayIntNullE"]
    static INTNULL_VTABLE: [*const c_void; 0];
    #[link_name = "_ZTVN5realm12ArrayIntegerE"]
    static INTEGER_VTABLE: [*const c_void; 0];
}

extern "C-unwind" {
    #[link_name = "_ZN5realm5Array6createENS_10NodeHeader4TypeEbNS1_9WidthTypeEmxRNS_9AllocatorE"]
    fn array_create(t: i32, ctx: bool, wt: i32, size: usize, value: i64, alloc: *mut Allocator) -> MemRef;
    #[link_name = "_ZN5realm5Array6insertEmx"]
    fn array_insert(this: *mut Array, ndx: usize, value: i64);
    #[link_name = "_ZN5realm5Array3setEmx"]
    fn array_set(this: *mut Array, ndx: usize, value: i64);
    #[link_name = "_ZN5realm5Array10set_as_refEmm"]
    fn array_set_as_ref(this: *mut Array, ndx: usize, ref_: usize);
    /// `realm::ArrayIntNull::create_array(Type, bool, size_t, Allocator&)`
    #[link_name = "_ZN5realm12ArrayIntNull12create_arrayENS_10NodeHeader4TypeEbmRNS_9AllocatorE"]
    fn intnull_create_array(t: i32, ctx: bool, size: usize, alloc: *mut Allocator) -> MemRef;
    /// `realm::ArrayIntNull::avoid_null_collision(int64_t)` — the one genuinely hard
    /// piece of the null encoding, and the reason none of it had to be reimplemented.
    #[link_name = "_ZN5realm12ArrayIntNull20avoid_null_collisionEx"]
    fn intnull_avoid_null_collision(this: *mut ArrayIntNull, value: i64);
}

extern "C" {
    #[link_name = "_ZN5realm5Array13init_from_memENS_6MemRefE"]
    fn array_init_from_mem(this: *mut Array, mem: MemRef);
    #[link_name = "_ZN5realm12ArrayIntNull16init_from_parentEv"]
    fn intnull_init_from_parent(this: *mut ArrayIntNull);
    /// `realm::ArrayIntNull::find_first(std::optional<int64_t>, size_t, size_t) const`
    #[link_name = "_ZNK5realm12ArrayIntNull10find_firstENSt3__18optionalIxEEmm"]
    fn intnull_find_first(this: *const ArrayIntNull, v: OptI64, begin: usize, end: usize) -> usize;
    #[link_name = "_ZNK5realm12ArrayIntNull19find_first_in_rangeExxmm"]
    fn intnull_find_first_in_range(this: *const ArrayIntNull, from: i64, to: i64, begin: usize, end: usize) -> usize;
    #[link_name = "_ZNK5realm12ArrayIntNull10find_firstINS_5EqualEEEmNSt3__18optionalIxEEmm"]
    fn intnull_find_first_equal(this: *const ArrayIntNull, v: OptI64, begin: usize, end: usize) -> usize;
    #[link_name = "_ZNK5realm12ArrayIntNull10find_firstINS_8NotEqualEEEmNSt3__18optionalIxEEmm"]
    fn intnull_find_first_notequal(this: *const ArrayIntNull, v: OptI64, begin: usize, end: usize) -> usize;
    #[link_name = "_ZNK5realm12ArrayIntNull10find_firstINS_12GreaterEqualEEEmNSt3__18optionalIxEEmm"]
    fn intnull_find_first_ge(this: *const ArrayIntNull, v: OptI64, begin: usize, end: usize) -> usize;
    #[link_name = "_ZNK5realm12ArrayIntNull10find_firstINS_9LessEqualEEEmNSt3__18optionalIxEEmm"]
    fn intnull_find_first_le(this: *const ArrayIntNull, v: OptI64, begin: usize, end: usize) -> usize;
}

// ---------------------------------------------------------------------------
// ArrayIntNull's inline encoding: index + 1, with the sentinel in element 0.
// ---------------------------------------------------------------------------

#[inline]
unsafe fn in_arr(a: *const ArrayIntNull) -> *const Array {
    core::ptr::addr_of!((*a).arr)
}

#[inline]
unsafe fn in_arr_mut(a: *mut ArrayIntNull) -> *mut Array {
    core::ptr::addr_of_mut!((*a).arr)
}

/// `ArrayIntNull::null_value()` — `Array::get(0)`.
#[inline]
unsafe fn intnull_null_value(a: *const ArrayIntNull) -> i64 {
    array_get(in_arr(a), 0)
}

/// `ArrayIntNull::size()` — one less than the backing array, which carries the sentinel.
#[inline]
unsafe fn intnull_size(a: *const ArrayIntNull) -> usize {
    (*in_arr(a)).m_size - 1
}

/// `ArrayIntNull::get(ndx)`.
#[inline]
unsafe fn intnull_get(a: *const ArrayIntNull, ndx: usize) -> OptI64 {
    let v = array_get(in_arr(a), ndx + 1);
    if v == intnull_null_value(a) {
        OptI64::NONE
    } else {
        OptI64::some(v)
    }
}

/// `ArrayIntNull::set(ndx, value)`.
#[inline]
unsafe fn intnull_set(a: *mut ArrayIntNull, ndx: usize, value: OptI64) {
    if value.engaged {
        // Must run before the store: if the value equals the current sentinel, the
        // sentinel is moved and the whole array rewritten.
        intnull_avoid_null_collision(a, value.value);
        array_set(in_arr_mut(a), ndx + 1, value.value);
    } else {
        let nv = intnull_null_value(a);
        array_set(in_arr_mut(a), ndx + 1, nv);
    }
}

/// `ArrayIntNull::set_null(ndx)`.
#[inline]
unsafe fn intnull_set_null(a: *mut ArrayIntNull, ndx: usize) {
    let nv = intnull_null_value(a);
    array_set(in_arr_mut(a), ndx + 1, nv);
}

/// `ArrayIntNull::insert(ndx, value)`.
#[inline]
unsafe fn intnull_insert(a: *mut ArrayIntNull, ndx: usize, value: OptI64) {
    if value.engaged {
        intnull_avoid_null_collision(a, value.value);
        array_insert(in_arr_mut(a), ndx + 1, value.value);
    } else {
        let nv = intnull_null_value(a);
        array_insert(in_arr_mut(a), ndx + 1, nv);
    }
}

/// `ArrayInteger::get(ndx)` — a plain `Array::get`, no sentinel involved.
#[inline]
unsafe fn integer_get(a: *const ArrayIntNull, ndx: usize) -> i64 {
    array_get(in_arr(a), ndx)
}

/// `ArrayInteger::set` / `insert`.
#[inline]
unsafe fn integer_set(a: *mut ArrayIntNull, ndx: usize, v: i64) {
    array_set(in_arr_mut(a), ndx, v);
}

#[inline]
unsafe fn integer_insert(a: *mut ArrayIntNull, ndx: usize, v: i64) {
    array_insert(in_arr_mut(a), ndx, v);
}

/// `Array::init_from_parent()` for the nanoseconds array — `init_from_ref(
/// get_ref_from_parent())`, where the ref comes through `ArrayParent::get_child_ref`
/// (virtual, measured slot 2, `adj = 0`).
#[inline]
unsafe fn integer_init_from_parent(a: *mut ArrayIntNull) {
    type GetChildRef = unsafe extern "C" fn(*mut c_void, usize) -> usize;
    let arr = in_arr_mut(a);
    let parent = (*arr).m_parent;
    let vptr = *(parent as *const *const GetChildRef);
    let f = *vptr.add(vslots::GET_CHILD_REF);
    let ref_ = f(parent, (*arr).m_ndx_in_parent as usize);
    let header = crate::array_unsigned::allocator_translate((*arr).m_alloc, ref_);
    array_init_from_mem(arr, MemRef { addr: header, ref_ });
}

// ---------------------------------------------------------------------------
// Construction: nine vtable pointers across three objects.
// ---------------------------------------------------------------------------

unsafe fn construct(this: *mut ArrayTimestamp, alloc: *mut Allocator) {
    core::ptr::write_bytes(this as *mut u8, 0, core::mem::size_of::<ArrayTimestamp>());

    let tsv = TS_VTABLE.as_ptr();
    let base = core::ptr::addr_of_mut!((*this).base);
    (*this).vptr_payload = tsv.add(vslots::TS_PAYLOAD) as *const c_void;
    (*base).vptr_node = tsv.add(vslots::TS_NODE) as *const c_void;
    (*base).vptr_array_parent = tsv.add(vslots::TS_ARRAY_PARENT) as *const c_void;
    (*base).m_alloc = alloc;

    // ArrayIntNull m_seconds(a), ArrayInteger m_nanoseconds(a)
    for (sub, vt) in [
        (core::ptr::addr_of_mut!((*this).m_seconds), INTNULL_VTABLE.as_ptr()),
        (core::ptr::addr_of_mut!((*this).m_nanoseconds), INTEGER_VTABLE.as_ptr()),
    ] {
        let a = in_arr_mut(sub);
        (*a).vptr_node = vt.add(vslots::SUB_NODE) as *const c_void;
        (*a).vptr_array_parent = vt.add(vslots::SUB_ARRAY_PARENT) as *const c_void;
        (*sub).vptr_payload = vt.add(vslots::SUB_PAYLOAD) as *const c_void;
        (*a).m_alloc = alloc;
    }

    // m_seconds.set_parent(this, 0); m_nanoseconds.set_parent(this, 1);
    // `this` becomes an ArrayParent* at +64 -- a pointer adjustment, not a cast.
    let as_parent = (this as *mut u8).add(TS_ARRAY_PARENT_OFFSET) as *mut c_void;
    let s = in_arr_mut(core::ptr::addr_of_mut!((*this).m_seconds));
    (*s).m_parent = as_parent;
    (*s).m_ndx_in_parent = 0;
    let n = in_arr_mut(core::ptr::addr_of_mut!((*this).m_nanoseconds));
    (*n).m_parent = as_parent;
    (*n).m_ndx_in_parent = 1;
}

// ===========================================================================
// The fourteen exported symbols.
// ===========================================================================

#[export_name = "_ZN5realm14ArrayTimestampC1ERNS_9AllocatorE"]
pub unsafe extern "C" fn ts_ctor_c1(this: *mut ArrayTimestamp, alloc: *mut Allocator) {
    construct(this, alloc);
}

#[export_name = "_ZN5realm14ArrayTimestampC2ERNS_9AllocatorE"]
pub unsafe extern "C" fn ts_ctor_c2(this: *mut ArrayTimestamp, alloc: *mut Allocator) {
    construct(this, alloc);
}

/// `realm::ArrayTimestamp::create()`
#[export_name = "_ZN5realm14ArrayTimestamp6createEv"]
pub unsafe extern "C-unwind" fn ts_create(this: *mut ArrayTimestamp) {
    let base = core::ptr::addr_of_mut!((*this).base);
    let alloc = (*base).m_alloc;

    // Array::create(type_HasRefs, false, 2) -- a two-element ref array.
    let mem = array_create(TYPE_HAS_REFS, false, WTYPE_BITS, 2, 0, alloc);
    array_init_from_mem(base, mem);

    let seconds = intnull_create_array(TYPE_NORMAL, false, 0, alloc);
    array_set_as_ref(base, 0, seconds.ref_);
    // ArrayInteger::create_empty_array(type_Normal, false, alloc)
    let nanos = array_create(TYPE_NORMAL, false, WTYPE_BITS, 0, 0, alloc);
    array_set_as_ref(base, 1, nanos.ref_);

    intnull_init_from_parent(core::ptr::addr_of_mut!((*this).m_seconds));
    integer_init_from_parent(core::ptr::addr_of_mut!((*this).m_nanoseconds));
}

/// `realm::ArrayTimestamp::init_from_mem(MemRef)`
#[export_name = "_ZN5realm14ArrayTimestamp13init_from_memENS_6MemRefE"]
pub unsafe extern "C" fn ts_init_from_mem(this: *mut ArrayTimestamp, mem: MemRef) {
    array_init_from_mem(core::ptr::addr_of_mut!((*this).base), mem);
    intnull_init_from_parent(core::ptr::addr_of_mut!((*this).m_seconds));
    integer_init_from_parent(core::ptr::addr_of_mut!((*this).m_nanoseconds));
}

/// `realm::ArrayTimestamp::set(size_t, Timestamp)`
#[export_name = "_ZN5realm14ArrayTimestamp3setEmNS_9TimestampE"]
pub unsafe extern "C-unwind" fn ts_set(this: *mut ArrayTimestamp, ndx: usize, value: Timestamp) {
    let secs = core::ptr::addr_of_mut!((*this).m_seconds);
    let nanos = core::ptr::addr_of_mut!((*this).m_nanoseconds);
    if value.m_is_null {
        // set_null(ndx) writes the sentinel into m_seconds and leaves m_nanoseconds
        // untouched -- mirrored, including that asymmetry.
        intnull_set_null(secs, ndx);
        return;
    }
    intnull_set(secs, ndx, OptI64::some(value.m_seconds));
    integer_set(nanos, ndx, value.m_nanoseconds as i64);
}

/// `realm::ArrayTimestamp::insert(size_t, Timestamp)`
#[export_name = "_ZN5realm14ArrayTimestamp6insertEmNS_9TimestampE"]
pub unsafe extern "C-unwind" fn ts_insert(this: *mut ArrayTimestamp, ndx: usize, value: Timestamp) {
    let secs = core::ptr::addr_of_mut!((*this).m_seconds);
    let nanos = core::ptr::addr_of_mut!((*this).m_nanoseconds);
    if value.m_is_null {
        intnull_insert(secs, ndx, OptI64::NONE);
        integer_insert(nanos, ndx, 0);
    } else {
        intnull_insert(secs, ndx, OptI64::some(value.m_seconds));
        integer_insert(nanos, ndx, value.m_nanoseconds as i64);
    }
}

/// `realm::ArrayTimestamp::verify() const` — body is entirely `#ifdef REALM_DEBUG`.
#[export_name = "_ZNK5realm14ArrayTimestamp6verifyEv"]
pub unsafe extern "C" fn ts_verify(_this: *const ArrayTimestamp) {}

// --- the six find_first specialisations -------------------------------------
//
// All six share a shape: narrow on the seconds array with the corresponding integer
// comparator, then break the tie on nanoseconds, then advance past the candidate.
// They are written out rather than factored behind a comparator enum, because the
// null handling differs between them and the tie-break operators are not symmetric:
// Greater/Less return `not_found` for a null needle, while GreaterEqual/LessEqual/
// Equal search for nulls and NotEqual searches for non-nulls.

#[export_name = "_ZNK5realm14ArrayTimestamp10find_firstINS_7GreaterEEEmNS_9TimestampEmm"]
pub unsafe extern "C" fn ts_find_first_greater(
    this: *const ArrayTimestamp,
    value: Timestamp,
    mut begin: usize,
    end: usize,
) -> usize {
    if value.m_is_null {
        return NOT_FOUND;
    }
    let secs = core::ptr::addr_of!((*this).m_seconds);
    let nanos = core::ptr::addr_of!((*this).m_nanoseconds);
    let sec = value.m_seconds;
    while begin < end {
        let ret = intnull_find_first_ge(secs, OptI64::some(sec), begin, end);
        if ret == NOT_FOUND {
            return NOT_FOUND;
        }
        let s = intnull_get(secs, ret);
        if s.value > sec {
            return ret;
        }
        let n = integer_get(nanos, ret) as i32;
        if n > value.m_nanoseconds {
            return ret;
        }
        begin = ret + 1;
    }
    NOT_FOUND
}

#[export_name = "_ZNK5realm14ArrayTimestamp10find_firstINS_4LessEEEmNS_9TimestampEmm"]
pub unsafe extern "C" fn ts_find_first_less(
    this: *const ArrayTimestamp,
    value: Timestamp,
    mut begin: usize,
    end: usize,
) -> usize {
    if value.m_is_null {
        return NOT_FOUND;
    }
    let secs = core::ptr::addr_of!((*this).m_seconds);
    let nanos = core::ptr::addr_of!((*this).m_nanoseconds);
    let sec = value.m_seconds;
    while begin < end {
        let ret = intnull_find_first_le(secs, OptI64::some(sec), begin, end);
        if ret == NOT_FOUND {
            return NOT_FOUND;
        }
        let s = intnull_get(secs, ret);
        if s.value < sec {
            return ret;
        }
        let n = integer_get(nanos, ret) as i32;
        if n < value.m_nanoseconds {
            return ret;
        }
        begin = ret + 1;
    }
    NOT_FOUND
}

#[export_name = "_ZNK5realm14ArrayTimestamp10find_firstINS_12GreaterEqualEEEmNS_9TimestampEmm"]
pub unsafe extern "C" fn ts_find_first_ge(
    this: *const ArrayTimestamp,
    value: Timestamp,
    mut begin: usize,
    end: usize,
) -> usize {
    let secs = core::ptr::addr_of!((*this).m_seconds);
    let nanos = core::ptr::addr_of!((*this).m_nanoseconds);
    if value.m_is_null {
        return intnull_find_first_equal(secs, OptI64::NONE, begin, end);
    }
    let sec = value.m_seconds;
    while begin < end {
        let ret = intnull_find_first_ge(secs, OptI64::some(sec), begin, end);
        if ret == NOT_FOUND {
            return NOT_FOUND;
        }
        let s = intnull_get(secs, ret);
        if s.value > sec {
            return ret;
        }
        let n = integer_get(nanos, ret) as i32;
        if n >= value.m_nanoseconds {
            return ret;
        }
        begin = ret + 1;
    }
    NOT_FOUND
}

#[export_name = "_ZNK5realm14ArrayTimestamp10find_firstINS_9LessEqualEEEmNS_9TimestampEmm"]
pub unsafe extern "C" fn ts_find_first_le(
    this: *const ArrayTimestamp,
    value: Timestamp,
    mut begin: usize,
    end: usize,
) -> usize {
    let secs = core::ptr::addr_of!((*this).m_seconds);
    let nanos = core::ptr::addr_of!((*this).m_nanoseconds);
    if value.m_is_null {
        return intnull_find_first_equal(secs, OptI64::NONE, begin, end);
    }
    let sec = value.m_seconds;
    while begin < end {
        let ret = intnull_find_first_le(secs, OptI64::some(sec), begin, end);
        if ret == NOT_FOUND {
            return NOT_FOUND;
        }
        let s = intnull_get(secs, ret);
        if s.value < sec {
            return ret;
        }
        let n = integer_get(nanos, ret) as i32;
        if n <= value.m_nanoseconds {
            return ret;
        }
        begin = ret + 1;
    }
    NOT_FOUND
}

#[export_name = "_ZNK5realm14ArrayTimestamp10find_firstINS_5EqualEEEmNS_9TimestampEmm"]
pub unsafe extern "C" fn ts_find_first_equal(
    this: *const ArrayTimestamp,
    value: Timestamp,
    mut begin: usize,
    end: usize,
) -> usize {
    let secs = core::ptr::addr_of!((*this).m_seconds);
    let nanos = core::ptr::addr_of!((*this).m_nanoseconds);
    if value.m_is_null {
        return intnull_find_first_equal(secs, OptI64::NONE, begin, end);
    }
    while begin < end {
        // The non-template overload, which is the plain equality search.
        let res = intnull_find_first(secs, OptI64::some(value.m_seconds), begin, end);
        if res == NOT_FOUND {
            return NOT_FOUND;
        }
        if integer_get(nanos, res) == value.m_nanoseconds as i64 {
            return res;
        }
        begin = res + 1;
    }
    NOT_FOUND
}

#[export_name = "_ZNK5realm14ArrayTimestamp10find_firstINS_8NotEqualEEEmNS_9TimestampEmm"]
pub unsafe extern "C" fn ts_find_first_notequal(
    this: *const ArrayTimestamp,
    value: Timestamp,
    mut begin: usize,
    end: usize,
) -> usize {
    let secs = core::ptr::addr_of!((*this).m_seconds);
    let nanos = core::ptr::addr_of!((*this).m_nanoseconds);
    if value.m_is_null {
        return intnull_find_first_notequal(secs, OptI64::NONE, begin, end);
    }
    let sec = value.m_seconds;
    // No narrowing search here: NotEqual scans linearly, because the first element that
    // differs is the answer and an index search cannot skip ahead.
    while begin < end {
        let s = intnull_get(secs, begin);
        if !s.engaged || s.value != sec {
            return begin;
        }
        let n = integer_get(nanos, begin) as i32;
        if n != value.m_nanoseconds {
            return begin;
        }
        begin += 1;
    }
    NOT_FOUND
}

/// `realm::ArrayTimestamp::find_first_in_range(Timestamp, Timestamp, size_t, size_t) const`
#[export_name = "_ZNK5realm14ArrayTimestamp19find_first_in_rangeENS_9TimestampES1_mm"]
pub unsafe extern "C-unwind" fn ts_find_first_in_range(
    this: *const ArrayTimestamp,
    from: Timestamp,
    to: Timestamp,
    mut start: usize,
    end: usize,
) -> usize {
    let secs = core::ptr::addr_of!((*this).m_seconds);
    let nanos = core::ptr::addr_of!((*this).m_nanoseconds);
    while start < end {
        start = intnull_find_first_in_range(secs, from.m_seconds, to.m_seconds, start, end);
        if start != NOT_FOUND {
            let s = intnull_get(secs, start);
            let n = integer_get(nanos, start) as i32;
            // Mirrored exactly, including that these are `||` and not `&&`: the seconds
            // search has already bounded the range, so each clause only has to rule out
            // the boundary element whose nanoseconds fall outside.
            if (from.m_seconds < s.value || from.m_nanoseconds <= n)
                && (to.m_seconds > s.value || n <= to.m_nanoseconds)
            {
                return start;
            }
            start += 1;
        }
    }
    NOT_FOUND
}
