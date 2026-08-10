//! `upstream/src/realm/node.cpp` — `realm::Node`, the base of every array accessor.
//!
//! This is the most format-critical unit ported so far. Its five functions are where
//! **element width, header layout and capacity growth are decided**, which is what
//! `format-fidelity.md` opens on. It is also the second unit that is byte-visible *and
//! traced* (after `array_unsigned`): every trace allocates nodes, so `make diff-test`
//! judges this directly rather than through a differential.
//!
//! # The 8-byte header
//!
//! Reproduced from `node_header.hpp`. Byte 4 is the flags/width byte and is the one that
//! decides on-disk element width:
//!
//! ```text
//!   byte 0..3   capacity  (24 bits, stored >> 3)  h[0]<<19 | h[1]<<11 | h[2]<<3
//!   byte 4      bit 7  is_inner_bptree_node
//!               bit 6  has_refs
//!               bit 5  context_flag
//!               bits 4..3  width_type   (0 bits, 1 multiply, 2 ignore)
//!               bits 2..0  width, packed as log2 + 1
//!   byte 5..7   size      (24 bits)
//! ```
//!
//! `get_width_from_header` is `(1 << (h[4] & 7)) >> 1`, so a stored 0 means width 0 and a
//! stored `w` means `2^(w-1)`. `set_width_in_header` counts bits to invert it. Mirrored
//! exactly rather than re-derived — a "cleaner" width calculation that produces a
//! different byte is a compatibility break.
//!
//! # Why this needed `panic = "unwind"`
//!
//! `Allocator::alloc` and `Allocator::realloc_` are **inline** in `alloc.hpp` and throw
//! `LogicError` on a read-only allocator. Three of the five functions here inherit that
//! throw site without containing a `throw`. There is no out-of-line copy to bind — only
//! the virtual `do_alloc`/`do_realloc` — so the check and the throw are reproduced here,
//! and the throw needs an unwind-capable profile. See `migration/checks/throw_probe/`.
//!
//! # Measured, not assumed
//!
//! Layout via `clang -Xclang -fdump-record-layouts`; vtable slots via the Itanium
//! encoding of a pointer-to-virtual-member-function (odd `ptr` = 1 + byte offset), because
//! `-fdump-vtable-layouts` emits nothing for a TU that is not the key-function TU.
//!
//! ```text
//!   Node                sizeof 56   vptr 0, m_data 8, m_ref 16, m_alloc 24,
//!                                   m_size 32, m_parent 40, m_ndx_in_parent 48,
//!                                   m_missing_parent_update 52
//!   Allocator           sizeof 64   m_is_read_only at 56
//!   MemRef              sizeof 16   {char* addr, size_t ref} -> rax:rdx, never sret
//!   ArrayPayload        sizeof 8    vptr only
//!
//!   Node::calc_byte_len            vtable byte 16 (slot 2)   <- called virtually by alloc
//!   Node::calc_item_count          vtable byte 24 (slot 3)
//!   Allocator::do_alloc            vtable byte 24 (slot 3)
//!   Allocator::do_realloc          vtable byte 32 (slot 4)
//!   Allocator::do_free             vtable byte 40 (slot 5)
//!   ArrayParent::get_child_ref     vtable byte 16 (slot 2)
//!   ArrayParent::update_child_ref  vtable byte 24 (slot 3)
//! ```
//!
//! Two of those slots — `do_translate` at 48 and `get_child_ref` at 16 — were measured
//! independently by `array_unsigned.rs` and agree, which is the cross-check that the
//! declaration-order model is right.

use core::ffi::{c_char, c_int, c_void};

use crate::array_unsigned::{Allocator, ArrayParent, MemRef};

// ---------------------------------------------------------------------------
// Constants, from node_header.hpp and node.hpp
// ---------------------------------------------------------------------------

/// `NodeHeader::header_size`.
pub(crate) const HEADER_SIZE: usize = 8;
/// `Node::initial_capacity` — the total byte size of a new empty array.
const INITIAL_CAPACITY: usize = 128;
/// `realm::max_array_size` — 24-bit size field.
const MAX_ARRAY_SIZE: usize = 0x00ff_ffff;
/// `realm::max_array_payload_aligned` — the 24-bit capacity field is stored `>> 3`.
const MAX_ARRAY_PAYLOAD_ALIGNED: usize = 0x07ff_ffc0;

/// `NodeHeader::Type`
const TYPE_NORMAL: c_int = 0;
const TYPE_INNER_BPTREE_NODE: c_int = 1;

/// `NodeHeader::WidthType`
const WTYPE_BITS: c_int = 0;
const WTYPE_MULTIPLY: c_int = 1;
const WTYPE_IGNORE: c_int = 2;

/// Vtable byte offsets. A wrong index here is a call to the wrong method — heap
/// corruption, not a byte diff — so this is the single audited place they appear.
mod vtable_slots {
    /// `realm::Node::calc_byte_len(size_t, size_t) const`
    pub(super) const NODE_CALC_BYTE_LEN: usize = 16;
    /// `realm::Allocator::do_alloc(size_t)`
    pub(super) const ALLOCATOR_DO_ALLOC: usize = 24;
    /// `realm::Allocator::do_realloc(ref_type, char*, size_t, size_t)`
    pub(super) const ALLOCATOR_DO_REALLOC: usize = 32;
    /// `realm::Allocator::do_free(ref_type, char*)`
    pub(super) const ALLOCATOR_DO_FREE: usize = 40;
    /// `realm::ArrayParent::update_child_ref(size_t, ref_type)`
    pub(super) const ARRAY_PARENT_UPDATE_CHILD_REF: usize = 24;
}

/// `realm::Node` — 56 bytes, polymorphic, vptr at 0.
#[repr(C)]
pub struct Node {
    vptr: *const *const c_void,
    pub(crate) m_data: *mut c_char,
    pub(crate) m_ref: usize,
    pub(crate) m_alloc: *mut Allocator,
    pub(crate) m_size: usize,
    pub(crate) m_parent: *mut ArrayParent,
    pub(crate) m_ndx_in_parent: u32,
    pub(crate) m_missing_parent_update: bool,
}

const _: () = {
    assert!(core::mem::size_of::<Node>() == 56);
    assert!(core::mem::offset_of!(Node, m_data) == 8);
    assert!(core::mem::offset_of!(Node, m_ref) == 16);
    assert!(core::mem::offset_of!(Node, m_alloc) == 24);
    assert!(core::mem::offset_of!(Node, m_size) == 32);
    assert!(core::mem::offset_of!(Node, m_parent) == 40);
    assert!(core::mem::offset_of!(Node, m_ndx_in_parent) == 48);
    assert!(core::mem::offset_of!(Node, m_missing_parent_update) == 52);
};

/// `std::initializer_list<T>` — `{const T* begin, size_t size}`. `util::terminate` takes
/// one by rvalue reference, i.e. a pointer to a temporary; only the empty one is needed.
#[repr(C)]
struct InitializerList {
    begin: *const c_void,
    size: usize,
}

extern "C" {
    /// `realm::util::terminate(const char*, const char*, long, std::initializer_list<Printable>&&)`
    ///
    /// `REALM_ASSERT_RELEASE` and `REALM_UNREACHABLE` are under no `#if` and always call
    /// this, so they must be reproduced as calls rather than as a Rust panic — see the
    /// correction in `evidence-and-linkage.md`.
    #[link_name = "_ZN5realm4util9terminateEPKcS2_lOSt16initializer_listINS0_9PrintableEE"]
    fn realm_terminate(message: *const c_char, file: *const c_char, line: i64, values: *const InitializerList) -> !;

    fn memmove(dst: *mut c_void, src: *const c_void, n: usize) -> *mut c_void;
}

/// Exactly the strings the C++ emits — taken out of `node.cpp.o`'s `__cstring` section
/// rather than retyped, so the abort message is identical.
const ASSERT_MSG_MAX_ARRAY_SIZE: &[u8] = b"Assertion failed: init_size <= max_array_size\0";
const NODE_CPP_PATH: &[u8] = b"/Users/satishbabariya/Desktop/realm-rust/upstream/src/realm/node.cpp\0";

#[inline]
unsafe fn assert_release_max_array_size(cond: bool) {
    if !cond {
        let empty = InitializerList { begin: core::ptr::null(), size: 0 };
        realm_terminate(
            ASSERT_MSG_MAX_ARRAY_SIZE.as_ptr() as *const c_char,
            NODE_CPP_PATH.as_ptr() as *const c_char,
            76,
            &empty,
        );
    }
}

// ---------------------------------------------------------------------------
// NodeHeader — the on-disk header. node_header.hpp.
// ---------------------------------------------------------------------------

#[inline]
pub(crate) unsafe fn get_data_from_header(header: *mut c_char) -> *mut c_char {
    header.add(HEADER_SIZE)
}

#[inline]
pub(crate) unsafe fn get_header_from_data(data: *mut c_char) -> *mut c_char {
    data.sub(HEADER_SIZE)
}

#[inline]
unsafe fn get_capacity_from_header(header: *const c_char) -> usize {
    let h = header as *const u8;
    ((*h.add(0) as usize) << 19) + ((*h.add(1) as usize) << 11) + ((*h.add(2) as usize) << 3)
}

#[inline]
unsafe fn set_capacity_in_header(value: usize, header: *mut c_char) {
    let h = header as *mut u8;
    *h.add(0) = ((value >> 19) & 0xFF) as u8;
    *h.add(1) = ((value >> 11) & 0xFF) as u8;
    *h.add(2) = ((value >> 3) & 0xFF) as u8;
}

/// `(1 << (h[4] & 7)) >> 1` — a stored 0 means width 0, a stored `w` means `2^(w-1)`.
#[inline]
unsafe fn get_width_from_header(header: *const c_char) -> usize {
    let h = header as *const u8;
    ((1usize << ((*h.add(4) as usize) & 0x07)) >> 1) as u8 as usize
}

/// The inverse: count bits until `value` is exhausted. `value` is `int` in the C++ and the
/// loop shifts it right, so a negative width would loop forever there too — mirrored as an
/// arithmetic shift on `i32` rather than "improved".
#[inline]
unsafe fn set_width_in_header(mut value: c_int, header: *mut c_char) {
    let mut w: c_int = 0;
    while value != 0 {
        w += 1;
        value >>= 1;
    }
    let h = header as *mut u8;
    *h.add(4) = (((*h.add(4) as c_int) & !0x7) | w) as u8;
}

#[inline]
unsafe fn get_wtype_from_header(header: *const c_char) -> c_int {
    let h = header as *const u8;
    ((*h.add(4) as c_int) & 0x18) >> 3
}

#[inline]
unsafe fn set_wtype_in_header(value: c_int, header: *mut c_char) {
    let h = header as *mut u8;
    *h.add(4) = (((*h.add(4) as c_int) & !0x18) | (value << 3)) as u8;
}

#[inline]
unsafe fn set_is_inner_bptree_node_in_header(value: bool, header: *mut c_char) {
    let h = header as *mut u8;
    *h.add(4) = (((*h.add(4) as c_int) & !0x80) | ((value as c_int) << 7)) as u8;
}

#[inline]
unsafe fn set_hasrefs_in_header(value: bool, header: *mut c_char) {
    let h = header as *mut u8;
    *h.add(4) = (((*h.add(4) as c_int) & !0x40) | ((value as c_int) << 6)) as u8;
}

#[inline]
unsafe fn set_context_flag_in_header(value: bool, header: *mut c_char) {
    let h = header as *mut u8;
    *h.add(4) = (((*h.add(4) as c_int) & !0x20) | ((value as c_int) << 5)) as u8;
}

#[inline]
unsafe fn set_size_in_header(value: usize, header: *mut c_char) {
    let h = header as *mut u8;
    *h.add(5) = ((value >> 16) & 0xFF) as u8;
    *h.add(6) = ((value >> 8) & 0xFF) as u8;
    *h.add(7) = (value & 0xFF) as u8;
}

/// `NodeHeader::calc_byte_size` — the width-to-bytes rule, then 8-byte alignment, then the
/// header. The `wtype_Bits` case relies on `size < 2^24` and `width <= 64` so the
/// multiply cannot overflow; that is upstream's stated assumption, kept as `wrapping_*`
/// because the release build wraps rather than trapping.
#[inline]
fn calc_byte_size(wtype: c_int, size: usize, width: u8) -> usize {
    let mut num_bytes: usize = match wtype {
        WTYPE_BITS => {
            let num_bits = size.wrapping_mul(width as usize);
            (num_bits.wrapping_add(7)) >> 3
        }
        WTYPE_MULTIPLY => size.wrapping_mul(width as usize),
        WTYPE_IGNORE => size,
        _ => 0,
    };
    num_bytes = (num_bytes.wrapping_add(7)) & !7usize;
    num_bytes + HEADER_SIZE
}

/// `Node::init_header` — node.hpp:362. Zeroes the whole header first, because it contains
/// unallocated bits that must be in a defined state.
#[inline]
unsafe fn init_header(
    header: *mut c_char,
    is_inner_bptree_node: bool,
    has_refs: bool,
    context_flag: bool,
    width_type: c_int,
    width: c_int,
    size: usize,
    capacity: usize,
) {
    core::ptr::write_bytes(header as *mut u8, 0, HEADER_SIZE);
    set_is_inner_bptree_node_in_header(is_inner_bptree_node, header);
    set_hasrefs_in_header(has_refs, header);
    set_context_flag_in_header(context_flag, header);
    set_wtype_in_header(width_type, header);
    set_width_in_header(width, header);
    set_size_in_header(size, header);
    set_capacity_in_header(capacity, header);
}

// ---------------------------------------------------------------------------
// Allocator — the inline members, including the two that throw.
// ---------------------------------------------------------------------------

#[inline]
unsafe fn allocator_vfn(alloc: *const Allocator, byte_offset: usize) -> *const c_void {
    let vptr = *(alloc as *const *const *const c_void);
    *vptr.add(byte_offset / 8)
}

/// `Allocator::alloc` — `alloc.hpp:493`. **Throws** `LogicError` on a read-only allocator.
#[inline]
unsafe fn allocator_alloc(alloc: *mut Allocator, size: usize) -> MemRef {
    if crate::exceptions::allocator_is_read_only_flag(alloc) {
        crate::exceptions::throw_wrong_transaction_state();
    }
    let f: unsafe extern "C-unwind" fn(*mut Allocator, usize) -> MemRef =
        core::mem::transmute(allocator_vfn(alloc, vtable_slots::ALLOCATOR_DO_ALLOC));
    f(alloc, size)
}

/// `Allocator::realloc_` — `alloc.hpp:501`. **Throws** `LogicError` on a read-only
/// allocator. The `REALM_DEBUG` watch branch above the check is compiled out here.
#[inline]
unsafe fn allocator_realloc(
    alloc: *mut Allocator,
    ref_: usize,
    addr: *const c_char,
    old_size: usize,
    new_size: usize,
) -> MemRef {
    if crate::exceptions::allocator_is_read_only_flag(alloc) {
        crate::exceptions::throw_wrong_transaction_state();
    }
    let f: unsafe extern "C-unwind" fn(*mut Allocator, usize, *mut c_char, usize, usize) -> MemRef =
        core::mem::transmute(allocator_vfn(alloc, vtable_slots::ALLOCATOR_DO_REALLOC));
    f(alloc, ref_, addr as *mut c_char, old_size, new_size)
}

/// `Allocator::free_` — `alloc.hpp:513`, `noexcept`.
#[inline]
unsafe fn allocator_free(alloc: *mut Allocator, ref_: usize, addr: *const c_char) {
    let f: unsafe extern "C" fn(*mut Allocator, usize, *mut c_char) =
        core::mem::transmute(allocator_vfn(alloc, vtable_slots::ALLOCATOR_DO_FREE));
    f(alloc, ref_, addr as *mut c_char)
}

// ---------------------------------------------------------------------------
// Node inline members this unit uses
// ---------------------------------------------------------------------------

/// `Node::update_parent` — node.hpp:257.
#[inline]
unsafe fn update_parent(this: *mut Node) {
    let parent = (*this).m_parent;
    if !parent.is_null() {
        let vptr = *(parent as *const *const *const c_void);
        let f: unsafe extern "C-unwind" fn(*mut ArrayParent, usize, usize) = core::mem::transmute(
            *vptr.add(vtable_slots::ARRAY_PARENT_UPDATE_CHILD_REF / 8),
        );
        f(parent, (*this).m_ndx_in_parent as usize, (*this).m_ref);
    } else {
        (*this).m_missing_parent_update = true;
    }
}

/// `Node::calc_byte_len` dispatched **virtually** on `this`, which is what
/// `Node::alloc` does — subclasses override it.
#[inline]
unsafe fn virtual_calc_byte_len(this: *const Node, num_items: usize, width: usize) -> usize {
    let vptr = (*this).vptr;
    let f: unsafe extern "C-unwind" fn(*const Node, usize, usize) -> usize =
        core::mem::transmute(*vptr.add(vtable_slots::NODE_CALC_BYTE_LEN / 8));
    f(this, num_items, width)
}

// ---------------------------------------------------------------------------
// Exported surface
// ---------------------------------------------------------------------------

/// `realm::Node::create_node(size_t, Allocator&, bool, Type, WidthType, int)`
///
/// Static, so no `this`. `MemRef` is 16 bytes and trivially copyable, hence `rax:rdx`.
#[no_mangle]
pub unsafe extern "C-unwind" fn _ZN5realm4Node11create_nodeEmRNS_9AllocatorEbNS_10NodeHeader4TypeENS3_9WidthTypeEi(
    size: usize,
    alloc: *mut Allocator,
    context_flag: bool,
    type_: c_int,
    width_type: c_int,
    width: c_int,
) -> MemRef {
    let byte_size_0 = calc_byte_size(width_type, size, width as u8);
    let byte_size = core::cmp::max(byte_size_0, INITIAL_CAPACITY);

    let mem = allocator_alloc(alloc, byte_size); // Throws
    let header = mem.addr;

    init_header(
        header,
        type_ == TYPE_INNER_BPTREE_NODE,
        type_ != TYPE_NORMAL,
        context_flag,
        width_type,
        width,
        size,
        byte_size,
    );

    mem
}

/// `realm::Node::calc_byte_len(size_t, size_t) const` — virtual, slot 2.
///
/// Returns the *unaligned* byte size, unlike `calc_byte_size`. The C++ carries a FIXME
/// wondering whether it should return the aligned one; it does not, and the difference is
/// visible in the capacity field, so it is kept.
#[no_mangle]
pub unsafe extern "C-unwind" fn _ZNK5realm4Node13calc_byte_lenEmm(
    _this: *const Node,
    num_items: usize,
    width: usize,
) -> usize {
    // REALM_ASSERT_3(get_wtype_from_header(...), ==, wtype_Bits) is a no-op in this build.
    let bits = num_items.wrapping_mul(width);
    let bytes = (bits.wrapping_add(7)) / 8; // round up
    bytes + HEADER_SIZE
}

/// `realm::Node::calc_item_count(size_t, size_t) const noexcept` — virtual, slot 3.
#[no_mangle]
pub unsafe extern "C" fn _ZNK5realm4Node15calc_item_countEmm(
    _this: *const Node,
    bytes: usize,
    width: usize,
) -> usize {
    if width == 0 {
        return usize::MAX; // Zero width gives "infinite" space
    }
    let bytes_data = bytes - HEADER_SIZE;
    let total_bits = bytes_data * 8;
    total_bits / width
}

/// `realm::Node::alloc(size_t, size_t)`
///
/// The capacity growth rule lives here and is part of the file format: double, clamp to
/// `max_array_payload_aligned`, and if doubling still is not enough, round the needed size
/// up to the next 8-byte boundary. Mirrored including the `(~needed & 7) + 1` idiom.
#[no_mangle]
pub unsafe extern "C-unwind" fn _ZN5realm4Node5allocEmm(this: *mut Node, init_size: usize, new_width: usize) {
    // REALM_ASSERT(is_attached()) is a no-op in this build.
    let needed_bytes = virtual_calc_byte_len(this, init_size, new_width);
    // Callers must ensure needed_bytes never exceeds max_array_payload; this one is
    // REALM_ASSERT_RELEASE and is *not* compiled out.
    assert_release_max_array_size(init_size <= MAX_ARRAY_SIZE);

    if crate::exceptions::allocator_is_read_only_ref((*this).m_alloc, (*this).m_ref) {
        _ZN5realm4Node16do_copy_on_writeEm(this, needed_bytes);
    }

    // REALM_ASSERT(!m_alloc.is_read_only(m_ref)) is a no-op in this build.
    let mut header = get_header_from_data((*this).m_data);
    let orig_capacity_bytes = get_capacity_from_header(header);
    let orig_width = get_width_from_header(header);

    if orig_capacity_bytes < needed_bytes {
        // Double to avoid too many reallocs, but truncate at the maximum payload the
        // 24-bit capacity field can express.
        let mut new_capacity_bytes = orig_capacity_bytes.wrapping_mul(2);
        if new_capacity_bytes < orig_capacity_bytes {
            // overflow detected, clamp to max
            new_capacity_bytes = MAX_ARRAY_PAYLOAD_ALIGNED;
        }
        if new_capacity_bytes > MAX_ARRAY_PAYLOAD_ALIGNED {
            new_capacity_bytes = MAX_ARRAY_PAYLOAD_ALIGNED;
        }

        // If doubling is not enough, expand enough to fit.
        if new_capacity_bytes < needed_bytes {
            let rest = (!needed_bytes & 0x7) + 1;
            new_capacity_bytes = needed_bytes;
            if rest < 8 {
                new_capacity_bytes += rest; // 64bit align
            }
        }

        let mem_ref = allocator_realloc(
            (*this).m_alloc,
            (*this).m_ref,
            header,
            orig_capacity_bytes,
            new_capacity_bytes,
        ); // Throws

        header = mem_ref.addr;
        set_capacity_in_header(new_capacity_bytes, header);

        (*this).m_ref = mem_ref.ref_;
        (*this).m_data = get_data_from_header(header);
        update_parent(this); // Throws
    }

    if new_width != orig_width {
        set_width_in_header(new_width as c_int, header);
    }
    set_size_in_header(init_size, header);
    (*this).m_size = init_size;
}

/// `realm::Node::do_copy_on_write(size_t)`
///
/// The `+ 64` of "matchcount room" is part of the on-disk capacity of every copied array,
/// so it is format, not a heuristic.
#[no_mangle]
pub unsafe extern "C-unwind" fn _ZN5realm4Node16do_copy_on_writeEm(this: *mut Node, minimum_size: usize) {
    let header = get_header_from_data((*this).m_data);

    let array_size = calc_byte_size(
        get_wtype_from_header(header),
        (*this).m_size,
        get_width_from_header(header) as u8,
    );
    let mut new_size = core::cmp::max(array_size, minimum_size);
    new_size = (new_size + 0x7) & !0x7usize; // 64bit blocks
    new_size += 64; // matchcount room for expansion

    let mref = allocator_alloc((*this).m_alloc, new_size); // Throws
    let old_begin = header;
    let old_end = header.add(array_size);
    let new_begin = mref.addr;
    // realm::safe_copy_n, which exists only to tolerate null pointers at count 0.
    memmove(
        new_begin as *mut c_void,
        old_begin as *const c_void,
        old_end as usize - old_begin as usize,
    );

    let old_ref = (*this).m_ref;

    (*this).m_ref = mref.ref_;
    (*this).m_data = get_data_from_header(new_begin);

    // Uses m_data to find the header, so m_data must be set first.
    set_capacity_in_header(new_size, new_begin);

    update_parent(this);

    // REALM_ENABLE_MEMDEBUG's 0x77 poisoning is compiled out in this build.

    // Mark the original as deleted so the space can be reclaimed in a later commit.
    allocator_free((*this).m_alloc, old_ref, old_begin);
}

// ---------------------------------------------------------------------------
// ArrayPayload::~ArrayPayload — an empty virtual destructor in three ABI variants.
// ---------------------------------------------------------------------------

extern "C-unwind" {
    #[link_name = "_ZdlPv"]
    fn cxx_operator_delete(p: *mut u8);
}

/// D2, base-object destructor. The C++ body is `{}`.
#[no_mangle]
pub unsafe extern "C-unwind" fn _ZN5realm12ArrayPayloadD2Ev(_this: *mut c_void) {}

/// D1, complete-object destructor.
#[no_mangle]
pub unsafe extern "C-unwind" fn _ZN5realm12ArrayPayloadD1Ev(_this: *mut c_void) {}

/// D0, deleting destructor: run the body, then `operator delete`.
#[no_mangle]
pub unsafe extern "C-unwind" fn _ZN5realm12ArrayPayloadD0Ev(this: *mut c_void) {
    cxx_operator_delete(this as *mut u8);
}
