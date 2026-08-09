//! Port of `upstream/src/realm/array_blobs_small.cpp`.
//!
//! **Byte-visible and traced.** `ArraySmallBlobs::insert` is hit 46 times and `set` 4
//! times across the five traces — it is how short strings reach the file. Chosen over
//! the regenerated queue's #1 (`array_timestamp`, live and portable but reached by no
//! trace) for exactly that reason.
//!
//! # Three coordinated arrays
//!
//! A small-blob node is one `Array` holding three refs, plus three accessors embedded
//! in the object:
//!
//! ```text
//! m_offsets  1, 1, 5, 5, 6      end offset of each element, cumulative
//! m_blob     aabcab             the bytes, concatenated
//! m_nulls    0, 0, 0, 1, 0      1 = null
//! ```
//!
//! Every mutation has to keep all three consistent: write the bytes, shift every
//! *later* offset by the size delta, and set the null flag. A port that updates two of
//! the three produces a file that reads back plausibly and is wrong somewhere else.
//!
//! # The largest object so far — `sizeof = 448`
//!
//! ```text
//!   0 | class realm::Array          the base: 112 bytes, two vptrs
//! 112 |   Array     m_offsets       112 bytes, two vptrs
//! 224 |   ArrayBlob m_blob          112 bytes, two vptrs  (already Rust)
//! 336 |   Array     m_nulls         112 bytes, two vptrs
//! ```
//!
//! Four `Array`-shaped subobjects. The ABI ground was broken by `array_blob` — the
//! 112-byte two-vptr layout, the `&vtable[2]` / `&vtable[10]` offsets, and the +56
//! `ArrayParent` adjustment — so this unit reuses all of it rather than re-deriving it.
//!
//! Only `create_array` constructs anything: it is `static`, so it builds a local `Array
//! top`. Every other function receives a `this` whose four subobjects were constructed
//! by the C++ caller, and must not re-initialise them.
//!
//! # Nearly everything it needs is callable
//!
//! `Array::get(header, ndx)` and `Array::get_two(header, ndx)` are **static and
//! out-of-line**, as are `Array::init_from_mem`, `set`, `insert`, `move`, `create` and
//! `destroy_children`. `ArrayBlob::replace` is this crate's own. So the Rust here is
//! mostly the coordination logic, which is the part that decides bytes.

use core::ffi::c_char;

use crate::array_blob::{
    array_add, array_get, array_get_as_ref, array_init_from_ref, new_array, replace_impl, Array,
    BinaryData, TYPE_HAS_REFS, TYPE_NORMAL, WTYPE_BITS, WTYPE_IGNORE,
};
use crate::array_unsigned::{allocator_translate, get_data_from_header, Allocator, MemRef};

/// `realm::ArraySmallBlobs` — `Array` plus three embedded accessors.
#[repr(C)]
pub struct ArraySmallBlobs {
    base: Array,      // 0
    m_offsets: Array, // 112
    m_blob: Array,    // 224 (an ArrayBlob; identical layout)
    m_nulls: Array,   // 336
}

/// `realm::StringData` — same shape as `BinaryData`, distinct type in C++.
#[repr(C)]
pub struct StringData {
    data: *const c_char,
    size: usize,
}

/// `std::pair<int64_t, int64_t>` as returned by `Array::get_two`.
#[repr(C)]
pub struct PairI64 {
    first: i64,
    second: i64,
}

const _: () = {
    assert!(core::mem::size_of::<ArraySmallBlobs>() == 448);
    assert!(core::mem::offset_of!(ArraySmallBlobs, m_offsets) == 112);
    assert!(core::mem::offset_of!(ArraySmallBlobs, m_blob) == 224);
    assert!(core::mem::offset_of!(ArraySmallBlobs, m_nulls) == 336);
    assert!(core::mem::size_of::<StringData>() == 16);
    assert!(core::mem::size_of::<PairI64>() == 16);
};

extern "C-unwind" {
    #[link_name = "_ZN5realm5Array6createENS_10NodeHeader4TypeEbNS1_9WidthTypeEmxRNS_9AllocatorE"]
    fn array_create(
        type_: i32,
        context_flag: bool,
        width_type: i32,
        size: usize,
        value: i64,
        alloc: *mut Allocator,
    ) -> MemRef;

    #[link_name = "_ZN5realm5Array6insertEmx"]
    fn array_insert_raw(this: *mut Array, ndx: usize, value: i64);

    #[link_name = "_ZN5realm5Array3setEmx"]
    fn array_set_raw(this: *mut Array, ndx: usize, value: i64);

    /// `realm::Array::move(size_t begin, size_t end, size_t dest_begin)`
    #[link_name = "_ZN5realm5Array4moveEmmm"]
    fn array_move(this: *mut Array, begin: usize, end: usize, dest_begin: usize);
}

extern "C" {
    #[link_name = "_ZN5realm5Array13init_from_memENS_6MemRefE"]
    fn array_init_from_mem(this: *mut Array, mem: MemRef);

    /// `realm::Array::get(const char* header, size_t ndx)` — static.
    #[link_name = "_ZN5realm5Array3getEPKcm"]
    fn array_get_from_header(header: *const c_char, ndx: usize) -> i64;

    /// `realm::Array::get_two(const char* header, size_t ndx)` — static.
    #[link_name = "_ZN5realm5Array7get_twoEPKcm"]
    fn array_get_two(header: *const c_char, ndx: usize) -> PairI64;
}

const NOT_FOUND: usize = usize::MAX; // realm::not_found == npos == size_t(-1)

// ---------------------------------------------------------------------------
// Inline Array/ArrayBlob helpers with no out-of-line definition.
// ---------------------------------------------------------------------------

/// `Node::set_header_size(value)` — `set_size_in_header(value, get_header())`.
#[inline]
unsafe fn set_header_size(a: *mut Array, value: usize) {
    let header = (a.read().m_data as *mut u8).sub(8);
    *header.add(5) = ((value >> 16) & 0xFF) as u8;
    *header.add(6) = ((value >> 8) & 0xFF) as u8;
    *header.add(7) = (value & 0xFF) as u8;
}

/// `Array::erase(ndx)` — `move(ndx + 1, size(), ndx)` then shrink the header.
#[inline]
unsafe fn array_erase(a: *mut Array, ndx: usize) {
    array_move(a, ndx + 1, (*a).m_size, ndx);
    (*a).m_size -= 1;
    set_header_size(a, (*a).m_size);
}

/// `Array::adjust(begin, end, diff)` — `set(i, get(i) + diff)` for each i, and a no-op
/// when `diff == 0`. Mirrored as a loop rather than "optimised": the C++ comment says
/// FIXME, but each `set` can widen the array, and widening is what the file records.
#[inline]
unsafe fn array_adjust(a: *mut Array, begin: usize, end: usize, diff: i64) {
    if diff != 0 {
        let mut i = begin;
        while i != end {
            let v = array_get(a, i);
            array_set_raw(a, i, v.wrapping_add(diff));
            i += 1;
        }
    }
}

/// `Array::back()` — `get(m_size - 1)`.
#[inline]
unsafe fn array_back(a: *const Array) -> i64 {
    array_get(a, (*a).m_size - 1)
}

/// `Array::insert(ndx, value)`.
#[inline]
unsafe fn array_insert(a: *mut Array, ndx: usize, value: i64) {
    array_insert_raw(a, ndx, value);
}

/// `ArrayBlob::get(header, pos)` — static: data start plus the offset.
#[inline]
unsafe fn array_blob_get_from_header(header: *const c_char, pos: usize) -> *const c_char {
    get_data_from_header(header as *mut u8).add(pos) as *const c_char
}

/// `Array::create(Type)` — the member overload, via `create_array(type, false, 0, 0)`.
#[inline]
unsafe fn array_create_type(a: *mut Array, type_: i32) {
    let mem = array_create(type_, false, WTYPE_BITS, 0, 0, (*a).m_alloc);
    array_init_from_mem(a, mem);
}

/// `Node::get_mem()` — `MemRef(get_header_from_data(m_data), m_ref)`.
#[inline]
unsafe fn array_get_mem(a: *const Array) -> MemRef {
    MemRef {
        addr: ((*a).m_data as *mut u8).sub(8) as *mut c_char,
        ref_: (*a).m_ref,
    }
}

#[inline]
fn to_size_t(v: i64) -> usize {
    v as usize
}

/// `BinaryData::is_null()` — `!m_data`.
#[inline]
fn bd_is_null(b: &BinaryData) -> bool {
    b.data.is_null()
}

// ===========================================================================
// The nine exported symbols.
// ===========================================================================

/// `realm::ArraySmallBlobs::init_from_mem(MemRef)`
#[export_name = "_ZN5realm15ArraySmallBlobs13init_from_memENS_6MemRefE"]
pub unsafe extern "C" fn asb_init_from_mem(this: *mut ArraySmallBlobs, mem: MemRef) {
    let base = core::ptr::addr_of_mut!((*this).base);
    array_init_from_mem(base, mem);
    let offsets_ref = array_get_as_ref(base, 0);
    let blob_ref = array_get_as_ref(base, 1);
    let nulls_ref = array_get_as_ref(base, 2);

    array_init_from_ref(core::ptr::addr_of_mut!((*this).m_offsets), offsets_ref);
    array_init_from_ref(core::ptr::addr_of_mut!((*this).m_blob), blob_ref);
    array_init_from_ref(core::ptr::addr_of_mut!((*this).m_nulls), nulls_ref);
}

/// `realm::ArraySmallBlobs::add(BinaryData, bool)`
#[export_name = "_ZN5realm15ArraySmallBlobs3addENS_10BinaryDataEb"]
pub unsafe extern "C-unwind" fn asb_add(
    this: *mut ArraySmallBlobs,
    value: BinaryData,
    add_zero_term: bool,
) {
    let offsets = core::ptr::addr_of_mut!((*this).m_offsets);
    let blob = core::ptr::addr_of_mut!((*this).m_blob);
    let nulls = core::ptr::addr_of_mut!((*this).m_nulls);

    // ArrayBlob::add(data, size, zt) == replace(m_size, m_size, ...)
    replace_impl(blob, (*blob).m_size, (*blob).m_size, value.data, value.size, add_zero_term);

    let mut end = value.size;
    if add_zero_term {
        end += 1;
    }
    // Offsets are cumulative, so a new element's end is the previous end plus its size.
    if (*offsets).m_size != 0 {
        end += to_size_t(array_back(offsets));
    }
    array_add(offsets, end as i64);
    array_add(nulls, bd_is_null(&value) as i64);
}

/// `realm::ArraySmallBlobs::set(size_t, BinaryData, bool)`
#[export_name = "_ZN5realm15ArraySmallBlobs3setEmNS_10BinaryDataEb"]
pub unsafe extern "C-unwind" fn asb_set(
    this: *mut ArraySmallBlobs,
    ndx: usize,
    value: BinaryData,
    add_zero_term: bool,
) {
    let offsets = core::ptr::addr_of_mut!((*this).m_offsets);
    let blob = core::ptr::addr_of_mut!((*this).m_blob);
    let nulls = core::ptr::addr_of_mut!((*this).m_nulls);

    let start = if ndx != 0 { array_get(offsets, ndx - 1) } else { 0 };
    let current_end = array_get(offsets, ndx);
    let mut stored_size = value.size;
    if add_zero_term {
        stored_size += 1;
    }
    // Signed: the replacement can be shorter than what it replaces.
    let diff = (start + stored_size as i64) - current_end;

    replace_impl(
        blob,
        to_size_t(start),
        to_size_t(current_end),
        value.data,
        value.size,
        add_zero_term,
    );
    // Every offset from ndx onward shifts, including ndx's own end.
    array_adjust(offsets, ndx, (*offsets).m_size, diff);
    array_set_raw(nulls, ndx, bd_is_null(&value) as i64);
}

/// `realm::ArraySmallBlobs::insert(size_t, BinaryData, bool)`
#[export_name = "_ZN5realm15ArraySmallBlobs6insertEmNS_10BinaryDataEb"]
pub unsafe extern "C-unwind" fn asb_insert(
    this: *mut ArraySmallBlobs,
    ndx: usize,
    value: BinaryData,
    add_zero_term: bool,
) {
    let offsets = core::ptr::addr_of_mut!((*this).m_offsets);
    let blob = core::ptr::addr_of_mut!((*this).m_blob);
    let nulls = core::ptr::addr_of_mut!((*this).m_nulls);

    let pos = if ndx != 0 {
        to_size_t(array_get(offsets, ndx - 1))
    } else {
        0
    };
    // ArrayBlob::insert(pos, ...) == replace(pos, pos, ...)
    replace_impl(blob, pos, pos, value.data, value.size, add_zero_term);

    let mut stored_size = value.size;
    if add_zero_term {
        stored_size += 1;
    }
    array_insert(offsets, ndx, (pos + stored_size) as i64);
    // The new element's own offset is already correct; everything after it moves.
    array_adjust(offsets, ndx + 1, (*offsets).m_size, stored_size as i64);
    array_insert(nulls, ndx, bd_is_null(&value) as i64);
}

/// `realm::ArraySmallBlobs::erase(size_t)`
#[export_name = "_ZN5realm15ArraySmallBlobs5eraseEm"]
pub unsafe extern "C-unwind" fn asb_erase(this: *mut ArraySmallBlobs, ndx: usize) {
    let offsets = core::ptr::addr_of_mut!((*this).m_offsets);
    let blob = core::ptr::addr_of_mut!((*this).m_blob);
    let nulls = core::ptr::addr_of_mut!((*this).m_nulls);

    let start = if ndx != 0 {
        to_size_t(array_get(offsets, ndx - 1))
    } else {
        0
    };
    let end = to_size_t(array_get(offsets, ndx));

    // ArrayBlob::erase(begin, end) == replace(begin, end, nullptr, 0, false)
    replace_impl(blob, start, end, core::ptr::null(), 0, false);
    array_erase(offsets, ndx);
    // Note the order: erase first, then adjust from ndx over the *shortened* array.
    array_adjust(offsets, ndx, (*offsets).m_size, start as i64 - end as i64);
    array_erase(nulls, ndx);
}

/// `realm::ArraySmallBlobs::get(const char* header, size_t ndx, Allocator&)` — static.
///
/// Reads straight out of a header without constructing any accessor, which is why it
/// uses the static `Array::get`/`get_two` overloads throughout.
#[export_name = "_ZN5realm15ArraySmallBlobs3getEPKcmRNS_9AllocatorE"]
pub unsafe extern "C" fn asb_get_static(
    header: *const c_char,
    ndx: usize,
    alloc: *mut Allocator,
) -> BinaryData {
    let ref_val = array_get_from_header(header, 2);
    let nulls_header = allocator_translate(alloc, to_size_t(ref_val));
    let n = array_get_from_header(nulls_header, ndx);
    // Only 0 or 1 is ever written to m_nulls; the C++ asserts it and the assert is a
    // no-op here, so any other value is treated as null exactly as the C++ would.
    if n != 0 {
        return BinaryData {
            data: core::ptr::null(),
            size: 0,
        };
    }

    let p = array_get_two(header, 0);
    let offsets_header = allocator_translate(alloc, to_size_t(p.first));
    let blob_header = allocator_translate(alloc, to_size_t(p.second));

    let (begin, end) = if ndx != 0 {
        // One fetch for both neighbouring offsets, as the C++ does.
        let q = array_get_two(offsets_header, ndx - 1);
        (to_size_t(q.first), to_size_t(q.second))
    } else {
        (0usize, to_size_t(array_get_from_header(offsets_header, ndx)))
    };

    BinaryData {
        data: array_blob_get_from_header(blob_header, begin),
        size: end - begin,
    }
}

/// `realm::ArraySmallBlobs::create_array(size_t, Allocator&, BinaryData)` — static.
///
/// The C++ wraps each step in `_impl::DeepArrayDestroyGuard` / `DeepArrayRefDestroyGuard`
/// and `release()`s every one of them on the success path, so on that path the guards
/// are inert and are not reproduced. They matter only if an allocation throws, where
/// the C++ frees the partial tree and this port would leak it. That is the same
/// OOM-only divergence `panic = "abort"` already documents; it is recorded rather than
/// hidden, because a leak on OOM is a real difference even if unreachable in practice.
#[export_name = "_ZN5realm15ArraySmallBlobs12create_arrayEmRNS_9AllocatorENS_10BinaryDataE"]
pub unsafe extern "C-unwind" fn asb_create_array(
    size: usize,
    alloc: *mut Allocator,
    values: BinaryData,
) -> MemRef {
    let mut top_storage = core::mem::MaybeUninit::<Array>::uninit();
    let top = top_storage.as_mut_ptr();
    new_array(top, alloc);
    array_create_type(top, TYPE_HAS_REFS);

    // 1. offsets: `size` zeroes, bit-packed.
    let mem = array_create(TYPE_NORMAL, false, WTYPE_BITS, size, 0, alloc);
    array_add(top, mem.ref_ as i64);

    // 2. blob: empty, wtype_Ignore because a blob is bytes.
    let mem = array_create(TYPE_NORMAL, false, WTYPE_IGNORE, 0, 0, alloc);
    array_add(top, mem.ref_ as i64);

    // 3. nulls: always created, even for a non-nullable column. The fill value is 1
    //    when the seed value is null, so a pre-sized node reads back as all-null.
    let value = if bd_is_null(&values) { 1 } else { 0 };
    let mem = array_create(TYPE_NORMAL, false, WTYPE_BITS, size, value, alloc);
    array_add(top, mem.ref_ as i64);

    array_get_mem(top)
}

/// `realm::ArraySmallBlobs::find_first(BinaryData, bool is_string, size_t, size_t) const`
#[export_name = "_ZNK5realm15ArraySmallBlobs10find_firstENS_10BinaryDataEbmm"]
pub unsafe extern "C" fn asb_find_first(
    this: *const ArraySmallBlobs,
    value: BinaryData,
    is_string: bool,
    begin: usize,
    end: usize,
) -> usize {
    let offsets = core::ptr::addr_of!((*this).m_offsets);
    let blob = core::ptr::addr_of!((*this).m_blob);
    let nulls = core::ptr::addr_of!((*this).m_nulls);

    // ArraySmallBlobs::size() is m_offsets.size(), not Array::size().
    let sz = (*offsets).m_size;
    let end = if end == NOT_FOUND { sz } else { end };

    if bd_is_null(&value) {
        let mut i = begin;
        while i != end {
            if array_get(nulls, i) != 0 {
                return i;
            }
            i += 1;
        }
    } else {
        // Strings are stored zero-terminated even though the needle may not be, so the
        // stored length is one more than the needle's. Getting this wrong makes every
        // string lookup miss.
        let value_size = value.size;
        let full_size = if is_string { value_size + 1 } else { value_size };

        let mut start_ofs = if begin != 0 {
            to_size_t(array_get(offsets, begin - 1))
        } else {
            0
        };
        let mut i = begin;
        while i != end {
            let end_ofs = to_size_t(array_get(offsets, i));
            let this_size = end_ofs - start_ofs;
            if array_get(nulls, i) == 0 && this_size == full_size {
                let blob_value = (*blob).m_data.add(start_ofs);
                // std::equal over value_size bytes -- the terminator is not compared.
                if value_size == 0
                    || core::slice::from_raw_parts(blob_value as *const u8, value_size)
                        == core::slice::from_raw_parts(value.data as *const u8, value_size)
                {
                    return i;
                }
            }
            start_ofs = end_ofs;
            i += 1;
        }
    }

    NOT_FOUND
}

/// `realm::ArraySmallBlobs::get_string_legacy(size_t) const`
///
/// Reads nodes written before file format 10, where `m_nulls` held the inverse
/// convention: a **true** value meant not-null. The `Array::size() == 3` test
/// distinguishes the legacy shape, and it is `Array::size()` — the base's ref count —
/// not `ArraySmallBlobs::size()`, which is the element count.
#[export_name = "_ZNK5realm15ArraySmallBlobs17get_string_legacyEm"]
pub unsafe extern "C" fn asb_get_string_legacy(this: *const ArraySmallBlobs, ndx: usize) -> StringData {
    let base = core::ptr::addr_of!((*this).base);
    let offsets = core::ptr::addr_of!((*this).m_offsets);
    let blob = core::ptr::addr_of!((*this).m_blob);
    let nulls = core::ptr::addr_of!((*this).m_nulls);

    if (*base).m_size == 3 && array_get(nulls, ndx) == 0 {
        return StringData {
            data: core::ptr::null(),
            size: 0,
        };
    }
    let begin = if ndx != 0 {
        to_size_t(array_get(offsets, ndx - 1))
    } else {
        0
    };
    let end = to_size_t(array_get(offsets, ndx));
    StringData {
        data: (*blob).m_data.add(begin),
        // -1 drops the zero terminator. Underflows for a zero-length stored element,
        // exactly as the C++ does.
        size: (end - begin).wrapping_sub(1),
    }
}
