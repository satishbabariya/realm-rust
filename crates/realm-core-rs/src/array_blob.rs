//! Port of `upstream/src/realm/array_blob.cpp`.
//!
//! **Byte-visible and traced** — the second unit ever in that category, after
//! `array_unsigned`. Confirmed by breakpoint, not inference: `ArrayBlob::replace` is
//! reached by `smoke`, `string_widths` and `erase_churn`, called from
//! `ArraySmallBlobs::insert`. `make diff-test` genuinely judges this port.
//!
//! # `Array` is multiply inherited — two vtable pointers
//!
//! `ArrayUnsigned` was 64 bytes with one vptr. `Array : public Node, public ArrayParent`
//! is **112 bytes with two**, measured with `-fdump-record-layouts`:
//!
//! ```text
//!   0 | (Node vtable pointer)
//!   8 |   char*        m_data
//!  16 |   size_t       m_ref
//!  24 |   Allocator&   m_alloc
//!  32 |   size_t       m_size
//!  40 |   ArrayParent* m_parent
//!  48 |   unsigned     m_ndx_in_parent
//!  52 |   bool         m_missing_parent_update
//!  56 | (ArrayParent vtable pointer)      <- second base
//!  64 |   Getter       m_getter           (pointer-to-member, 16 B)
//!  80 |   const VTable* m_vtable
//!  88 |   int64_t      m_lbound
//!  96 |   int64_t      m_ubound
//! 104 |   uint8_t      m_width
//! 105 |   bool         m_is_inner_bptree_node
//! 106 |   bool         m_has_refs
//! 107 |   bool         m_context_flag
//! ```
//!
//! Two consequences the single-inheritance units never hit:
//!
//! 1. **Constructing one sets two vptrs.** `-fdump-vtable-layouts` emits nothing for a
//!    TU that is not the key-function TU (same obstacle as `array_unsigned`), so the
//!    offsets were measured from a real object: both classes put the primary vptr at
//!    `&vtable[2]` and the `ArrayParent` vptr at `&vtable[10]`.
//! 2. **Passing `this` as an `ArrayParent*` adds 56.** `blob_replace` does
//!    `lastNode.set_parent(this, …)`, and that conversion is a pointer adjustment, not
//!    a cast. Getting it wrong stores a pointer that looks valid and dispatches into
//!    the wrong vtable.
//!
//! # Why the reimplementation surface is small
//!
//! Splitting `nm -m` by linkage leaves **four strong symbols**; everything else this
//! unit defines is weak and coalesced. And most of what it needs is out-of-line and
//! therefore bindable: `Array::create`, `Array::insert`, `Array::init_from_mem`,
//! `Array::destroy_deep`, `Array::blob_size`, `Node::alloc`.
//!
//! What genuinely has no out-of-line definition anywhere in `librealm.a` — checked, not
//! assumed — is `Array::get`, `Array::get_as_ref`, `Array::add` and
//! `ArrayBlob::create_array`. All four are one-liners over things that *are* callable,
//! so they are reimplemented here rather than being a shared-layer blocker:
//!
//! - `Array::get(ndx)` is `(this->*m_getter)(ndx)`, and every `m_getter` target is
//!   `Array::get_universal<w>`, which is **byte-for-byte the same function** as
//!   `array_unsigned`'s `get_direct`. Reusing that avoids decoding a
//!   pointer-to-member-function entirely.
//! - `Array::add(v)` is `insert(m_size, v)`; `Array::insert` is out-of-line.
//! - `ArrayBlob::create_array(n, alloc)` is
//!   `Array::create(type_Normal, false, wtype_Ignore, n, 0, alloc)`; that static is
//!   out-of-line.

use core::ffi::{c_char, c_uint, c_void};

use crate::array_unsigned::{
    allocator_is_read_only, allocator_translate, get_direct, get_size_from_header, Allocator, MemRef,
};

/// `realm::Array` / `realm::ArrayBlob` — identical layout; `ArrayBlob` adds no members.
#[repr(C)]
pub struct Array {
    pub(crate) vptr_node: *const c_void,       // 0
    pub(crate) m_data: *mut c_char,            // 8
    pub(crate) m_ref: usize,                   // 16
    pub(crate) m_alloc: *mut Allocator,        // 24
    pub(crate) m_size: usize,                  // 32
    pub(crate) m_parent: *mut c_void,          // 40
    pub(crate) m_ndx_in_parent: c_uint,        // 48
    m_missing_parent_update: bool,  // 52
    // 53..56 padding
    vptr_array_parent: *const c_void, // 56
    m_getter: [usize; 2],           // 64  (pointer-to-member)
    m_vtable: *const c_void,        // 80
    m_lbound: i64,                  // 88
    m_ubound: i64,                  // 96
    pub(crate) m_width: u8,                    // 104
    m_is_inner_bptree_node: bool,   // 105
    m_has_refs: bool,               // 106
    pub(crate) m_context_flag: bool,           // 107
    // 108..112 padding
}

/// `realm::BinaryData` — `{const char*, size_t}`, returned in `rax:rdx`.
#[repr(C)]
pub struct BinaryData {
    pub(crate) data: *const c_char,
    pub(crate) size: usize,
}

const _: () = {
    assert!(core::mem::size_of::<Array>() == 112);
    assert!(core::mem::offset_of!(Array, m_data) == 8);
    assert!(core::mem::offset_of!(Array, m_ref) == 16);
    assert!(core::mem::offset_of!(Array, m_alloc) == 24);
    assert!(core::mem::offset_of!(Array, m_size) == 32);
    assert!(core::mem::offset_of!(Array, m_parent) == 40);
    assert!(core::mem::offset_of!(Array, m_ndx_in_parent) == 48);
    // The one a single-inheritance layout would miss entirely.
    assert!(core::mem::offset_of!(Array, vptr_array_parent) == ARRAY_PARENT_SUBOBJECT_OFFSET);
    assert!(core::mem::offset_of!(Array, m_getter) == 64);
    assert!(core::mem::offset_of!(Array, m_vtable) == 80);
    assert!(core::mem::offset_of!(Array, m_width) == 104);
    assert!(core::mem::offset_of!(Array, m_context_flag) == 107);
    assert!(core::mem::size_of::<BinaryData>() == 16);
};

/// Byte offset of the `ArrayParent` base subobject inside `Array`.
pub(crate) const ARRAY_PARENT_SUBOBJECT_OFFSET: usize = 56;

/// `ArrayBlob::max_binary_size` = `0xFFFFF8 - Array::header_size`.
const MAX_BINARY_SIZE: usize = 0xFF_FFF8 - 8;

// NodeHeader::Type / WidthType, from node_header.hpp.
pub(crate) const TYPE_NORMAL: i32 = 0;
pub(crate) const TYPE_HAS_REFS: i32 = 2;
pub(crate) const WTYPE_BITS: i32 = 0;
pub(crate) const WTYPE_IGNORE: i32 = 2;

extern "C" {
    /// The `ArrayBlob` and `Array` vtable groups. Both are `weak external` with several
    /// definers, so removing this translation unit orphans neither — this port uses
    /// them, it does not synthesize them.
    #[link_name = "_ZTVN5realm9ArrayBlobE"]
    static ARRAY_BLOB_VTABLE: [*const c_void; 0];
    #[link_name = "_ZTVN5realm5ArrayE"]
    static ARRAY_VTABLE: [*const c_void; 0];
}

extern "C-unwind" {
    /// `realm::Array::create(Type, bool, WidthType, size_t, int_fast64_t, Allocator&)`
    #[link_name = "_ZN5realm5Array6createENS_10NodeHeader4TypeEbNS1_9WidthTypeEmxRNS_9AllocatorE"]
    fn array_create(
        type_: i32,
        context_flag: bool,
        width_type: i32,
        size: usize,
        value: i64,
        alloc: *mut Allocator,
    ) -> MemRef;

    /// `realm::Array::insert(size_t, int64_t)`
    #[link_name = "_ZN5realm5Array6insertEmx"]
    fn array_insert(this: *mut Array, ndx: usize, value: i64);

    /// `realm::Node::alloc(size_t, size_t)`
    #[link_name = "_ZN5realm4Node5allocEmm"]
    fn node_alloc(this: *mut Array, init_size: usize, new_width: usize);
}

extern "C" {
    /// `realm::Array::init_from_mem(MemRef)`
    #[link_name = "_ZN5realm5Array13init_from_memENS_6MemRefE"]
    fn array_init_from_mem(this: *mut Array, mem: MemRef);

    /// `realm::Array::destroy_deep()`
    #[link_name = "_ZN5realm5Array12destroy_deepEv"]
    fn array_destroy_deep(this: *mut Array);

    /// `realm::Array::blob_size() const`
    #[link_name = "_ZNK5realm5Array9blob_sizeEv"]
    fn array_blob_size(this: *const Array) -> usize;

    /// `realm::util::terminate(const char*, const char*, long, initializer_list<Printable>&&)`
    #[link_name = "_ZN5realm4util9terminateEPKcS2_lOSt16initializer_listINS0_9PrintableEE"]
    fn realm_terminate(msg: *const c_char, file: *const c_char, line: i64, infos: *mut InitializerList) -> !;
}

#[repr(C)]
struct InitializerList {
    begin: *const c_void,
    size: usize,
}

const THIS_FILE: &[u8] = b"upstream/src/realm/array_blob.cpp\0";

/// `REALM_UNREACHABLE()` — live in this build (`assert.hpp:99`, under no `#if`).
#[inline(never)]
unsafe fn unreachable(line: i64) -> ! {
    let mut empty = InitializerList {
        begin: core::ptr::null(),
        size: 0,
    };
    realm_terminate(c"Unreachable code".as_ptr(), THIS_FILE.as_ptr() as *const c_char, line, &mut empty)
}

// ---------------------------------------------------------------------------
// Construction of stack-local Array / ArrayBlob accessors.
// ---------------------------------------------------------------------------

/// Reproduces `Array(Allocator&) : Node(allocator)` and `ArrayBlob(Allocator&)`.
///
/// The C++ constructor sets both vptrs, binds `m_alloc`, and relies on default member
/// initialisers for `m_data`/`m_size`/`m_parent`/`m_getter`/`m_vtable`/`m_width`
/// (all null or zero). `m_ref`, `m_lbound` and `m_ubound` are left **indeterminate** by
/// C++ and nothing reads them before `init_from_mem`/`create` writes them; zeroing them
/// is the same determinism-over-indeterminacy choice made for `error_codes`'s pair
/// padding.
#[inline]
unsafe fn construct(out: *mut Array, alloc: *mut Allocator, vtable: *const *const c_void) {
    core::ptr::write_bytes(out as *mut u8, 0, core::mem::size_of::<Array>());
    (*out).vptr_node = vtable.add(2) as *const c_void;
    (*out).vptr_array_parent = vtable.add(10) as *const c_void;
    (*out).m_alloc = alloc;
}

#[inline]
pub(crate) unsafe fn new_array_blob(out: *mut Array, alloc: *mut Allocator) {
    construct(out, alloc, ARRAY_BLOB_VTABLE.as_ptr());
}

#[inline]
pub(crate) unsafe fn new_array(out: *mut Array, alloc: *mut Allocator) {
    construct(out, alloc, ARRAY_VTABLE.as_ptr());
}

// ---------------------------------------------------------------------------
// Array inline accessors with no out-of-line definition in librealm.a.
// ---------------------------------------------------------------------------

/// `Array::get(ndx)` — `(this->*m_getter)(ndx)`, whose targets are all
/// `Array::get_universal<w>`, which is the same function as `get_direct`.
#[inline]
pub(crate) unsafe fn array_get(this: *const Array, ndx: usize) -> i64 {
    get_direct((*this).m_data, (*this).m_width as usize, ndx)
}

/// `Array::get_as_ref(ndx)` — `to_ref(get(ndx))`. `to_ref` is a checked cast whose
/// assertion is compiled out here, leaving the conversion.
#[inline]
pub(crate) unsafe fn array_get_as_ref(this: *const Array, ndx: usize) -> usize {
    array_get(this, ndx) as usize
}

/// `Array::add(value)` — `insert(m_size, value)`.
#[inline]
pub(crate) unsafe fn array_add(this: *mut Array, value: i64) {
    array_insert(this, (*this).m_size, value);
}

/// `Node::init_from_ref(ref)` — translate then `init_from_mem`.
#[inline]
pub(crate) unsafe fn array_init_from_ref(this: *mut Array, ref_: usize) {
    let header = allocator_translate((*this).m_alloc, ref_);
    array_init_from_mem(this, MemRef { addr: header, ref_ });
}

/// `ArrayBlob::create()` — `create_array(0, alloc)` then `init_from_mem`.
#[inline]
pub(crate) unsafe fn array_blob_create(this: *mut Array) {
    let mem = array_create(TYPE_NORMAL, false, WTYPE_IGNORE, 0, 0, (*this).m_alloc);
    array_init_from_mem(this, mem);
}

/// `Array::create(Type, bool)` — the member overload, which goes through
/// `create_array(type, ctx, 0, 0, alloc)` and therefore `wtype_Bits`.
#[inline]
pub(crate) unsafe fn array_create_member(this: *mut Array, type_: i32, context_flag: bool) {
    let mem = array_create(type_, context_flag, WTYPE_BITS, 0, 0, (*this).m_alloc);
    array_init_from_mem(this, mem);
}

/// `Node::set_parent(ArrayParent*, size_t)`.
///
/// The caller passes an `Array*` where an `ArrayParent*` is expected. That conversion
/// is a **pointer adjustment of +56**, not a cast — `ArrayParent` is `Array`'s second
/// base. Handled at the call site so the adjustment is visible there.
#[inline]
pub(crate) unsafe fn array_set_parent(this: *mut Array, parent: *mut c_void, ndx_in_parent: usize) {
    (*this).m_parent = parent;
    (*this).m_ndx_in_parent = ndx_in_parent as c_uint;
}

/// `Array*` viewed as the `ArrayParent*` its second base occupies.
#[inline]
pub(crate) unsafe fn as_array_parent(this: *mut Array) -> *mut c_void {
    (this as *mut u8).add(ARRAY_PARENT_SUBOBJECT_OFFSET) as *mut c_void
}

/// `ArrayBlob::add(data, size, add_zero_term)` — `replace(m_size, m_size, …)`.
#[inline]
unsafe fn array_blob_add(this: *mut Array, data: *const c_char, data_size: usize, add_zero_term: bool) -> usize {
    replace_impl(this, (*this).m_size, (*this).m_size, data, data_size, add_zero_term)
}

/// Pointer to a static empty string, mirroring the `BinaryData{"", 0}` literals. The
/// C++ returns a pointer to a string literal, not null, and callers do distinguish.
static EMPTY: [u8; 1] = [0];

// ===========================================================================
// The four exported symbols.
// ===========================================================================

/// `realm::ArrayBlob::verify() const`
///
/// The entire body is inside `#ifdef REALM_DEBUG`, which is not defined in this build,
/// so the function is empty. It is still exported and still in `ArrayBlob`'s vtable.
#[export_name = "_ZNK5realm9ArrayBlob6verifyEv"]
pub unsafe extern "C" fn array_blob_verify(_this: *const Array) {}

/// `realm::ArrayBlob::get_at(size_t&) const`
#[export_name = "_ZNK5realm9ArrayBlob6get_atERm"]
pub unsafe extern "C" fn array_blob_get_at(this: *const Array, pos: *mut usize) -> BinaryData {
    let mut offset = *pos;

    if (*this).m_context_flag {
        // The root holds refs to child blobs; walk them until `offset` lands inside one.
        let mut ndx = 0usize;
        let mut current_size =
            get_size_from_header(allocator_translate((*this).m_alloc, array_get_as_ref(this, ndx)) as *const u8);

        while offset >= current_size {
            ndx += 1;
            if ndx >= (*this).m_size {
                *pos = 0;
                return BinaryData {
                    data: EMPTY.as_ptr() as *const c_char,
                    size: 0,
                };
            }
            offset -= current_size;
            current_size =
                get_size_from_header(allocator_translate((*this).m_alloc, array_get_as_ref(this, ndx)) as *const u8);
        }

        let mut blob = core::mem::MaybeUninit::<Array>::uninit();
        let blob = blob.as_mut_ptr();
        new_array_blob(blob, (*this).m_alloc);
        array_init_from_ref(blob, array_get_as_ref(this, ndx));
        ndx += 1;
        let sz = current_size - offset;

        // `pos` advances by the returned length unless this was the last child, in
        // which case it is reset to 0 to signal "no more".
        *pos = if ndx >= (*this).m_size { 0 } else { *pos + sz };

        BinaryData {
            data: (*blob).m_data.add(offset),
            size: sz,
        }
    } else {
        *pos = 0;
        if offset < (*this).m_size {
            BinaryData {
                data: (*this).m_data.add(offset),
                size: (*this).m_size - offset,
            }
        } else {
            BinaryData {
                data: EMPTY.as_ptr() as *const c_char,
                size: 0,
            }
        }
    }
}

/// Body of `ArrayBlob::replace`, shared with `ArrayBlob::add`.
pub(crate) unsafe fn replace_impl(
    this: *mut Array,
    begin: usize,
    end: usize,
    data: *const c_char,
    data_size: usize,
    add_zero_term: bool,
) -> usize {
    // Called for its value only in the REALM_ASSERTs, which are no-ops in this build.
    // The call is kept because blob_size() is out-of-line and mirroring the C++ call
    // sequence costs nothing; it is pure, so eliding it would also be sound.
    let _sz = array_blob_size(this);
    let remove_size = end - begin;
    let add_size = if add_zero_term { data_size + 1 } else { data_size };
    let old_size = (*this).m_size;
    let new_size = (*this).m_size - remove_size + add_size;

    // Above max_binary_size the data is split across child blobs and the root becomes
    // a ref-holding array. This is the only path in this function that constructs an
    // accessor, and no trace reaches it.
    if new_size > MAX_BINARY_SIZE {
        let mut new_root = core::mem::MaybeUninit::<Array>::uninit();
        let new_root = new_root.as_mut_ptr();
        new_array(new_root, (*this).m_alloc);
        array_create_member(new_root, TYPE_HAS_REFS, true);
        array_add(new_root, (*this).m_ref as i64);
        return blob_replace_impl(new_root, begin, end, data, data_size, add_zero_term);
    }

    // Unchanged content in a read-only node needs no copy-on-write at all.
    if remove_size == add_size
        && allocator_is_read_only((*this).m_alloc, (*this).m_ref)
        && (data_size == 0 || memcmp((*this).m_data.add(begin), data, data_size) == 0)
    {
        return (*this).m_ref;
    }

    // Reallocates if needed and updates the header. Width is 1: a blob is bytes.
    node_alloc(this, new_size, 1);

    // m_data is read *after* alloc, which may have relocated the node.
    let mut modify_begin = (*this).m_data.add(begin);

    if begin != old_size {
        let old_begin = (*this).m_data.add(end);
        let old_end = (*this).m_data.add(old_size);
        let tail = old_end as usize - old_begin as usize;
        if remove_size < add_size {
            // std::copy_backward(old_begin, old_end, m_data + new_size)
            let new_end = (*this).m_data.add(new_size);
            core::ptr::copy(old_begin, new_end.sub(tail), tail);
        } else if add_size < remove_size {
            // realm::safe_copy_n(old_begin, old_end - old_begin, modify_begin + add_size)
            core::ptr::copy(old_begin, modify_begin.add(add_size), tail);
        }
    }

    core::ptr::copy(data, modify_begin, data_size);
    modify_begin = modify_begin.add(data_size);
    if add_zero_term {
        *modify_begin = 0;
    }

    (*this).m_ref
}

/// `realm::ArrayBlob::replace(size_t, size_t, const char*, size_t, bool)`
#[export_name = "_ZN5realm9ArrayBlob7replaceEmmPKcmb"]
pub unsafe extern "C-unwind" fn array_blob_replace(
    this: *mut Array,
    begin: usize,
    end: usize,
    data: *const c_char,
    data_size: usize,
    add_zero_term: bool,
) -> usize {
    replace_impl(this, begin, end, data, data_size, add_zero_term)
}

/// Body of `Array::blob_replace`.
unsafe fn blob_replace_impl(
    this: *mut Array,
    begin: usize,
    end: usize,
    mut data: *const c_char,
    mut data_size: usize,
    add_zero_term: bool,
) -> usize {
    let sz = array_blob_size(this);

    if begin == sz && end == sz {
        // Append. Fill whatever room is left in the last child, then add children.
        let mut last = core::mem::MaybeUninit::<Array>::uninit();
        let last = last.as_mut_ptr();
        new_array_blob(last, (*this).m_alloc);
        array_init_from_ref(last, array_get_as_ref(this, (*this).m_size - 1));
        // `this` is an Array*; set_parent takes an ArrayParent*, which is +56.
        array_set_parent(last, as_array_parent(this), (*this).m_size - 1);

        let space_left = MAX_BINARY_SIZE - (*last).m_size;
        let size_to_copy = core::cmp::min(space_left, data_size);
        array_blob_add(last, data, size_to_copy, false);

        // NOTE: upstream subtracts and advances by `space_left`, not by `size_to_copy`.
        // When data_size < space_left this underflows and the loop below runs with a
        // huge count. Mirrored with wrapping arithmetic rather than "fixed": the
        // caller only reaches this path for data larger than a full node, so the
        // difference is unreachable in practice, and changing it would change which
        // nodes the data lands in — a file-format decision.
        data_size = data_size.wrapping_sub(space_left);
        data = data.wrapping_add(space_left);

        while data_size != 0 {
            let size_to_copy = core::cmp::min(MAX_BINARY_SIZE, data_size);
            let mut new_blob = core::mem::MaybeUninit::<Array>::uninit();
            let new_blob = new_blob.as_mut_ptr();
            new_array_blob(new_blob, (*this).m_alloc);
            array_blob_create(new_blob);

            let ref_ = array_blob_add(new_blob, data, size_to_copy, false);
            array_add(this, ref_ as i64);

            data_size -= size_to_copy;
            data = data.add(size_to_copy);
        }
        return (*this).m_ref;
    } else if begin == 0 && end == sz {
        // Replace everything: throw the tree away and build one fresh blob.
        array_destroy_deep(this);
        let mut new_blob = core::mem::MaybeUninit::<Array>::uninit();
        let new_blob = new_blob.as_mut_ptr();
        new_array_blob(new_blob, (*this).m_alloc);
        array_blob_create(new_blob);
        return array_blob_add(new_blob, data, data_size, add_zero_term);
    }

    // REALM_UNREACHABLE() at array_blob.cpp:114 — live in this build.
    unreachable(114)
}

/// `realm::Array::blob_replace(size_t, size_t, const char*, size_t, bool)`
#[export_name = "_ZN5realm5Array12blob_replaceEmmPKcmb"]
pub unsafe extern "C-unwind" fn array_blob_replace_root(
    this: *mut Array,
    begin: usize,
    end: usize,
    data: *const c_char,
    data_size: usize,
    add_zero_term: bool,
) -> usize {
    blob_replace_impl(this, begin, end, data, data_size, add_zero_term)
}

extern "C" {
    fn memcmp(a: *const c_char, b: *const c_char, n: usize) -> i32;
}
