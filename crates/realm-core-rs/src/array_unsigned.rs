//! Port of `upstream/src/realm/array_unsigned.cpp`.
//!
//! **The first unit whose bytes `make diff-test` can actually judge.** Every unit
//! ported before this one is byte-invisible or untraced, so the gate has never failed
//! on a real port. This one writes element-width-packed array payloads and header
//! bytes straight into the `.realm`, which is what the whole harness exists to check.
//!
//! # Layout, measured — not read off the header
//!
//! `clang -Xclang -fdump-record-layouts`. `Node` is **polymorphic**: there is a vtable
//! pointer at offset 0, so every field is shifted 8 bytes from what the declaration
//! order suggests, and `ArrayUnsigned::m_width` is packed into the base's tail padding
//! at 53.
//!
//! ```text
//! *** class realm::ArrayUnsigned                       [sizeof=64, align=8]
//!   0 | (Node vtable pointer)
//!   8 |   char*         m_data
//!  16 |   size_t        m_ref
//!  24 |   Allocator&    m_alloc                  (a reference: one pointer)
//!  32 |   size_t        m_size
//!  40 |   ArrayParent*  m_parent
//!  48 |   unsigned      m_ndx_in_parent
//!  52 |   bool          m_missing_parent_update
//!  53 | uint_least8_t   m_width
//!  56 | uint64_t        m_ubound
//! ```
//!
//! Rust never constructs or destroys one of these — it only operates on a `this`
//! pointer supplied by C++, so the vptr is simply a field it must skip.
//!
//! # Calling back into C++
//!
//! Five out-of-line symbols are bound directly. One of them,
//! `Allocator::translate_critical`, is `weak private external` — 35 copies in
//! `librealm.a`, and it becomes a *local* (`t`) symbol in the linked binary. That
//! looked unlinkable and it is not: Mach-O `N_PEXT` means "external during static
//! linking, made local in the output image". Verified by linking against it in the
//! hybrid's exact archive order before relying on it here.
//!
//! # Two virtual dispatches
//!
//! `update_from_parent()` reaches `ArrayParent::get_child_ref` and, on the cold path,
//! `Allocator::do_translate`. Slot indices were measured off the Itanium encoding of a
//! pointer-to-virtual-member-function (odd `ptr` = 1 + byte offset into the vtable),
//! because `-fdump-vtable-layouts` emits nothing for a TU that is not the
//! key-function TU. Both have `adj = 0`, so the call is a plain indirect through the
//! vptr with `this` unadjusted.
//!
//! A wrong slot index here is a call to the wrong method — heap corruption, not a byte
//! diff — so `VTABLE_SLOTS` below is the single audited place they appear, and
//! `migration/checks/run_array_unsigned_differential.sh` re-measures them at runtime
//! rather than trusting this comment.

use core::ffi::{c_char, c_int, c_uint, c_void};
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

// ---------------------------------------------------------------------------
// Measured vtable slot indices. See the module comment for how they were taken.
// ---------------------------------------------------------------------------
mod vtable_slots {
    /// `realm::ArrayParent::get_child_ref(size_t) const` — vtable byte offset 16.
    pub const ARRAY_PARENT_GET_CHILD_REF: usize = 2;
    /// `realm::Allocator::do_translate(ref_type) const` — vtable byte offset 48.
    pub const ALLOCATOR_DO_TRANSLATE: usize = 6;
}

/// Opaque: only ever reached through its vptr.
#[repr(C)]
pub struct ArrayParent {
    _vptr: *const c_void,
}

/// Prefix view of `realm::Allocator`. Only the first 32 bytes are described because
/// only those are touched; the real class is `sizeof = 64`. Never constructed here.
#[repr(C)]
pub struct Allocator {
    _vptr: *const c_void,                                  // 0
    m_baseline: AtomicUsize,                               // 8
    _m_debug_watch: usize,                                 // 16
    m_ref_translation_ptr: AtomicPtr<RefTranslation>,      // 24
}

/// Opaque — passed straight back to `translate_critical`/`translate_less_critical`.
/// Deliberately *not* described field by field: its layout is conditional on
/// `REALM_ENABLE_ENCRYPTION`, and nothing here needs to look inside it.
#[repr(C)]
pub struct RefTranslation {
    _opaque: [u8; 0],
}

/// `realm::MemRef` — `{char* m_addr; size_t m_ref}`, trivially copyable, so
/// `create_node` returns it in `rax:rdx` rather than through an `sret` pointer.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MemRef {
    pub(crate) addr: *mut c_char,
    pub(crate) ref_: usize,
}

/// `realm::ArrayUnsigned`. See the module comment for the measured offsets.
#[repr(C)]
pub struct ArrayUnsigned {
    _vptr: *const c_void,           // 0
    m_data: *mut c_char,            // 8
    m_ref: usize,                   // 16
    m_alloc: *mut Allocator,        // 24  (Allocator& is one pointer)
    m_size: usize,                  // 32
    _m_parent: *mut ArrayParent,    // 40
    _m_ndx_in_parent: c_uint,       // 48
    _m_missing_parent_update: bool, // 52
    m_width: u8,                    // 53
    // 54..56 padding
    m_ubound: u64,                  // 56
}

const _: () = {
    assert!(core::mem::size_of::<ArrayUnsigned>() == 64);
    assert!(core::mem::offset_of!(ArrayUnsigned, m_data) == 8);
    assert!(core::mem::offset_of!(ArrayUnsigned, m_ref) == 16);
    assert!(core::mem::offset_of!(ArrayUnsigned, m_alloc) == 24);
    assert!(core::mem::offset_of!(ArrayUnsigned, m_size) == 32);
    assert!(core::mem::offset_of!(ArrayUnsigned, _m_parent) == 40);
    assert!(core::mem::offset_of!(ArrayUnsigned, _m_ndx_in_parent) == 48);
    assert!(core::mem::offset_of!(ArrayUnsigned, _m_missing_parent_update) == 52);
    // The one that a hand-written layout gets wrong: m_width is packed into the tail
    // padding of Node, not placed after it.
    assert!(core::mem::offset_of!(ArrayUnsigned, m_width) == 53);
    assert!(core::mem::offset_of!(ArrayUnsigned, m_ubound) == 56);
    assert!(core::mem::size_of::<MemRef>() == 16);
    assert!(core::mem::offset_of!(Allocator, m_baseline) == 8);
    assert!(core::mem::offset_of!(Allocator, m_ref_translation_ptr) == 24);
};

// ---------------------------------------------------------------------------
// C++ symbols this port calls.
//
// `C-unwind` because `create_node`, `Node::alloc` and `do_copy_on_write` are all
// marked "// Throws" upstream and the ArrayUnsigned methods that call them are not
// `noexcept`. Under the workspace's `panic = "abort"` an escaping C++ exception
// aborts rather than propagating; that is a divergence on the allocation-failure
// path only, and it is the same one `base64` documented.
// ---------------------------------------------------------------------------
extern "C-unwind" {
    /// `realm::Node::create_node(size_t, Allocator&, bool, Type, WidthType, int)`
    #[link_name = "_ZN5realm4Node11create_nodeEmRNS_9AllocatorEbNS_10NodeHeader4TypeENS3_9WidthTypeEi"]
    fn node_create_node(
        size: usize,
        alloc: *mut Allocator,
        context_flag: bool,
        type_: c_int,
        width_type: c_int,
        width: c_int,
    ) -> MemRef;

    /// `realm::Node::do_copy_on_write(size_t)`
    #[link_name = "_ZN5realm4Node16do_copy_on_writeEm"]
    fn node_do_copy_on_write(this: *mut ArrayUnsigned, minimum_size: usize);

    /// `realm::Node::alloc(size_t, size_t)`
    #[link_name = "_ZN5realm4Node5allocEmm"]
    fn node_alloc(this: *mut ArrayUnsigned, init_size: usize, new_width: usize);
}

extern "C" {
    /// `realm::Allocator::translate_critical(RefTranslation*, ref_type) const`
    ///
    /// `weak private external`; see the module comment for why binding to it works.
    #[link_name = "_ZNK5realm9Allocator18translate_criticalEPNS0_14RefTranslationEm"]
    fn allocator_translate_critical(
        this: *const Allocator,
        txl: *mut RefTranslation,
        ref_: usize,
    ) -> *mut c_char;

    /// `realm::util::terminate(const char*, const char*, long, initializer_list<Printable>&&)`
    ///
    /// `REALM_UNREACHABLE()` expands to the 3-argument variadic overload, which
    /// forwards to this one with an empty list (`terminate.hpp:41-46`).
    #[link_name = "_ZN5realm4util9terminateEPKcS2_lOSt16initializer_listINS0_9PrintableEE"]
    fn realm_terminate(
        message: *const c_char,
        file: *const c_char,
        line: i64,
        infos: *mut InitializerList,
    ) -> !;
}

/// `std::initializer_list<Printable>` — `{const T* begin; size_t size}`. Passed by
/// rvalue reference, i.e. as a pointer to a temporary.
#[repr(C)]
struct InitializerList {
    begin: *const c_void,
    size: usize,
}

/// Mirror of `REALM_UNREACHABLE()` (`util/assert.hpp:99`).
///
/// This is under no `#if` — unlike `REALM_ASSERT*`, it is live in this build. It must
/// be a call to `realm::util::terminate`, not a Rust `panic!` and not
/// `unreachable_unchecked`: the former changes the failure mode, the latter turns a
/// defined abort into undefined behaviour.
#[inline(never)]
unsafe fn unreachable(file: &[u8], line: i64) -> ! {
    let mut empty = InitializerList {
        begin: core::ptr::null(),
        size: 0,
    };
    realm_terminate(
        c"Unreachable code".as_ptr(),
        file.as_ptr() as *const c_char,
        line,
        &mut empty,
    )
}

const THIS_FILE: &[u8] = b"upstream/src/realm/array_unsigned.cpp\0";

// ---------------------------------------------------------------------------
// NodeHeader — pure arithmetic, reimplemented from node_header.hpp.
// ---------------------------------------------------------------------------
const HEADER_SIZE: usize = 8;

#[inline]
pub(crate) unsafe fn get_data_from_header(header: *mut u8) -> *mut u8 {
    header.add(HEADER_SIZE)
}

#[inline]
unsafe fn get_header_from_data(data: *mut u8) -> *mut u8 {
    data.sub(HEADER_SIZE)
}

#[inline]
pub(crate) unsafe fn get_size_from_header(header: *const u8) -> usize {
    ((*header.add(5) as usize) << 16) + ((*header.add(6) as usize) << 8) + (*header.add(7) as usize)
}

/// `(1 << (h[4] & 0x07)) >> 1` — so the stored 3-bit log2 maps 0→0, 1→1, 2→2, 3→4,
/// 4→8 … 7→64. Note the `>> 1`: the encoding is off by one from a plain log2.
#[inline]
unsafe fn get_width_from_header(header: *const u8) -> u8 {
    ((1i32 << ((*header.add(4) as i32) & 0x07)) >> 1) as u8
}

#[inline]
unsafe fn set_size_in_header(value: usize, header: *mut u8) {
    *header.add(5) = ((value >> 16) & 0x0000_00FF) as u8;
    *header.add(6) = ((value >> 8) & 0x0000_00FF) as u8;
    *header.add(7) = (value & 0x0000_00FF) as u8;
}

/// Packs width as a 3-bit log2 by counting shifts, exactly as the C++ does. Not
/// `leading_zeros`-derived: for value 0 the C++ loop leaves `w = 0`, and for value 8
/// it yields 4, which is the off-by-one encoding `get_width_from_header` undoes.
#[inline]
unsafe fn set_width_in_header(mut value: c_int, header: *mut u8) {
    let mut w: c_int = 0;
    while value != 0 {
        w += 1;
        value >>= 1;
    }
    *header.add(4) = (((*header.add(4) as c_int) & !0x7) | w) as u8;
}

// ---------------------------------------------------------------------------
// array_direct.hpp — get_direct and the sub-byte lower/upper_bound.
// ---------------------------------------------------------------------------

/// `realm::get_direct<w>(const char* data, size_t ndx)`.
///
/// `data` is `const char*`, which is **signed** on x86_64 Darwin, so the C++ shifts
/// are arithmetic. For widths 1/2/4 the mask discards every sign-extended bit, so the
/// result is unaffected — mirrored with `i8` anyway rather than relying on that
/// reasoning being right. Width 8 is a *signed* char load in the C++; that path is
/// unreachable from `_get`, which handles 8/16/32 itself, but it is mirrored because
/// `lower_bound` reaches `get_direct` for other widths.
#[inline]
pub(crate) unsafe fn get_direct(data: *const c_char, width: usize, ndx: usize) -> i64 {
    let d = data as *const i8;
    match width {
        0 => 0,
        1 => {
            let offset = ndx >> 3;
            ((*d.add(offset) >> (ndx & 7)) & 0x01) as i64
        }
        2 => {
            let offset = ndx >> 2;
            ((*d.add(offset) >> ((ndx & 3) << 1)) & 0x03) as i64
        }
        4 => {
            let offset = ndx >> 1;
            ((*d.add(offset) >> ((ndx & 1) << 2)) & 0x0F) as i64
        }
        8 => *d.add(ndx) as i64,
        16 => (data.add(ndx * 2) as *const i16).read_unaligned() as i64,
        32 => (data.add(ndx * 4) as *const i32).read_unaligned() as i64,
        64 => (data.add(ndx * 8) as *const i64).read_unaligned(),
        // REALM_ASSERT_DEBUG(false) then `return int64_t(-1)`. The assert is a no-op
        // in this build, so -1 is the observable behaviour.
        _ => -1,
    }
}

/// `realm::lower_bound<width>(const char*, size_t, int64_t)` — array_direct.hpp:236.
///
/// The three-times-unrolled `size >= 8` block is kept. It is the same body repeated,
/// so for sorted input the result matches a plain loop; it is mirrored anyway because
/// "the same for sorted input" is an assumption about the caller, not about this
/// function.
///
/// The comparison is **signed** (`int64_t v < int64_t value`), and `ArrayUnsigned`
/// hands it a `uint64_t`. Values above `INT64_MAX` therefore compare as negative in
/// both stacks.
#[inline]
unsafe fn lower_bound_direct(data: *const c_char, mut size: usize, value: i64, width: usize) -> usize {
    let mut low = 0usize;
    macro_rules! step {
        () => {{
            let half = size / 2;
            let other_half = size - half;
            let probe = low + half;
            let other_low = low + other_half;
            let v = get_direct(data, width, probe);
            size = half;
            low = if v < value { other_low } else { low };
        }};
    }
    while size >= 8 {
        step!();
        step!();
        step!();
    }
    while size > 0 {
        step!();
    }
    low
}

/// `realm::upper_bound<width>` — array_direct.hpp:312. Same shape, `value >= v`.
#[inline]
unsafe fn upper_bound_direct(data: *const c_char, mut size: usize, value: i64, width: usize) -> usize {
    let mut low = 0usize;
    macro_rules! step {
        () => {{
            let half = size / 2;
            let other_half = size - half;
            let probe = low + half;
            let other_low = low + other_half;
            let v = get_direct(data, width, probe);
            size = half;
            low = if value >= v { other_low } else { low };
        }};
    }
    while size >= 8 {
        step!();
        step!();
        step!();
    }
    while size > 0 {
        step!();
    }
    low
}

/// `std::lower_bound` over a sorted array of unsigned elements: first index whose
/// element is **not less than** `value`.
///
/// The C++ compares `*it < value` where `*it` is `uintN_t` and `value` is `uint64_t`;
/// the usual arithmetic conversions make that an unsigned comparison at 64 bits, so
/// no sign extension is involved for any of 8/16/32/64.
#[inline]
unsafe fn std_lower_bound(data: *const c_char, size: usize, value: u64, elem_bytes: usize) -> usize {
    let mut low = 0usize;
    let mut count = size;
    while count > 0 {
        let half = count / 2;
        let probe = low + half;
        if load_unsigned(data, probe, elem_bytes) < value {
            low = probe + 1;
            count -= half + 1;
        } else {
            count = half;
        }
    }
    low
}

/// `std::upper_bound`: first index whose element is **greater than** `value`.
#[inline]
unsafe fn std_upper_bound(data: *const c_char, size: usize, value: u64, elem_bytes: usize) -> usize {
    let mut low = 0usize;
    let mut count = size;
    while count > 0 {
        let half = count / 2;
        let probe = low + half;
        if value >= load_unsigned(data, probe, elem_bytes) {
            low = probe + 1;
            count -= half + 1;
        } else {
            count = half;
        }
    }
    low
}

#[inline]
unsafe fn load_unsigned(data: *const c_char, ndx: usize, elem_bytes: usize) -> u64 {
    match elem_bytes {
        1 => *(data as *const u8).add(ndx) as u64,
        2 => (data.add(ndx * 2) as *const u16).read_unaligned() as u64,
        4 => (data.add(ndx * 4) as *const u32).read_unaligned() as u64,
        _ => (data.add(ndx * 8) as *const u64).read_unaligned(),
    }
}

// ---------------------------------------------------------------------------
// Allocator / Node inline helpers.
// ---------------------------------------------------------------------------

/// `Allocator::is_read_only(ref)` — `alloc.hpp:539`.
#[inline]
pub(crate) unsafe fn allocator_is_read_only(alloc: *const Allocator, ref_: usize) -> bool {
    ref_ < (*alloc).m_baseline.load(Ordering::Relaxed)
}

/// `Allocator::translate(ref)` — `alloc.hpp:568`.
#[inline]
pub(crate) unsafe fn allocator_translate(alloc: *mut Allocator, ref_: usize) -> *mut c_char {
    let ptr = (*alloc).m_ref_translation_ptr.load(Ordering::Acquire);
    if !ptr.is_null() {
        allocator_translate_critical(alloc, ptr, ref_)
    } else {
        // Virtual: Allocator::do_translate, measured slot 6, adj = 0.
        type DoTranslate = unsafe extern "C" fn(*mut Allocator, usize) -> *mut c_char;
        let vptr = *(alloc as *const *const DoTranslate);
        let f = *vptr.add(vtable_slots::ALLOCATOR_DO_TRANSLATE);
        f(alloc, ref_)
    }
}

/// `Node::get_ref_from_parent()` — `node.hpp:190`. Virtual:
/// `ArrayParent::get_child_ref`, measured slot 2, adj = 0.
#[inline]
unsafe fn get_ref_from_parent(this: *const ArrayUnsigned) -> usize {
    type GetChildRef = unsafe extern "C" fn(*mut ArrayParent, usize) -> usize;
    let parent = (*this)._m_parent;
    let vptr = *(parent as *const *const GetChildRef);
    let f = *vptr.add(vtable_slots::ARRAY_PARENT_GET_CHILD_REF);
    f(parent, (*this)._m_ndx_in_parent as usize)
}

#[inline]
unsafe fn get_header(this: *const ArrayUnsigned) -> *mut u8 {
    get_header_from_data((*this).m_data as *mut u8)
}

/// `Node::copy_on_write()` — `node.hpp:295`. `REALM_ENABLE_MEMDEBUG` is off, so the
/// live branch is `if (is_read_only()) do_copy_on_write();`, with the default
/// `minimum_size = 0`.
#[inline]
unsafe fn copy_on_write(this: *mut ArrayUnsigned) {
    if allocator_is_read_only((*this).m_alloc, (*this).m_ref) {
        node_do_copy_on_write(this, 0);
    }
}

/// `Node::init_from_mem(MemRef)` — `node.hpp:122`.
#[inline]
unsafe fn node_init_from_mem(this: *mut ArrayUnsigned, mem: MemRef) {
    let header = mem.addr as *mut u8;
    (*this).m_ref = mem.ref_;
    (*this).m_data = get_data_from_header(header) as *mut c_char;
    (*this).m_size = get_size_from_header(header);
}

/// `ArrayUnsigned::init_from_mem(MemRef)` — `array_unsigned.hpp:87`.
#[inline]
unsafe fn au_init_from_mem(this: *mut ArrayUnsigned, mem: MemRef) {
    node_init_from_mem(this, mem);
    set_width_impl(this, get_width_from_header(get_header(this)));
}

/// `ArrayUnsigned::alloc(size_t, size_t)` — `array_unsigned.hpp:100`.
#[inline]
unsafe fn au_alloc(this: *mut ArrayUnsigned, init_size: usize, new_width: usize) {
    node_alloc(this, init_size, new_width);
    set_width_impl(this, new_width as u8);
}

/// `ArrayUnsigned::bit_width(uint64_t)` — private inline, no emitted symbol.
#[inline]
fn bit_width(value: u64) -> u8 {
    if value < 0x100 {
        return 8;
    }
    if value < 0x10000 {
        return 16;
    }
    if value < 0x1_0000_0000 {
        return 32;
    }
    64
}

/// `ArrayUnsigned::_set(ndx, width, value)`.
#[inline]
unsafe fn _set(this: *mut ArrayUnsigned, ndx: usize, width: u8, value: u64) {
    let data = (*this).m_data;
    match width {
        8 => *(data as *mut u8).add(ndx) = value as u8,
        16 => (data.add(ndx * 2) as *mut u16).write_unaligned(value as u16),
        32 => (data.add(ndx * 4) as *mut u32).write_unaligned(value as u32),
        _ => (data.add(ndx * 8) as *mut u64).write_unaligned(value),
    }
}

/// `ArrayUnsigned::_get(ndx, width)`.
#[inline]
unsafe fn _get(this: *const ArrayUnsigned, ndx: usize, width: u8) -> u64 {
    let data = (*this).m_data;
    match width {
        8 => *(data as *const u8).add(ndx) as u64,
        16 => (data.add(ndx * 2) as *const u16).read_unaligned() as u64,
        32 => (data.add(ndx * 4) as *const u32).read_unaligned() as u64,
        // Everything else, including 64 and the sub-byte widths, goes through
        // get_direct exactly as the C++ does.
        _ => get_direct(data, width as usize, ndx) as u64,
    }
}

/// Body of `set_width`, shared with the exported symbol.
#[inline]
unsafe fn set_width_impl(this: *mut ArrayUnsigned, width: u8) {
    // `uint64_t(-1) >> (64 - width)`.
    //
    // For width == 0 that is a shift by 64, which is undefined in C++. On x86-64 the
    // `shr` instruction takes its count mod 64, so the observed behaviour is a shift
    // by 0 and m_ubound becomes UINT64_MAX. `wrapping_shr` reproduces exactly that
    // masking. A plain `>>` would panic — shift overflow is checked in Rust
    // regardless of `overflow-checks` — turning a working array into an abort.
    (*this).m_ubound = u64::MAX.wrapping_shr(64u32.wrapping_sub(width as u32));
    (*this).m_width = width;
}

// ===========================================================================
// The ten exported symbols.
// ===========================================================================

/// `realm::ArrayUnsigned::set_width(uint8_t)`
#[export_name = "_ZN5realm13ArrayUnsigned9set_widthEh"]
pub unsafe extern "C" fn array_unsigned_set_width(this: *mut ArrayUnsigned, width: u8) {
    set_width_impl(this, width);
}

/// `realm::ArrayUnsigned::create(size_t, uint64_t)`
#[export_name = "_ZN5realm13ArrayUnsigned6createEmy"]
pub unsafe extern "C-unwind" fn array_unsigned_create(
    this: *mut ArrayUnsigned,
    initial_size: usize,
    ubound_value: u64,
) {
    // type_Normal = 0, wtype_Bits = 0 (node_header.hpp:34-55).
    let mem = node_create_node(
        initial_size,
        (*this).m_alloc,
        false,
        0, // Node::type_Normal
        0, // wtype_Bits
        bit_width(ubound_value) as c_int,
    );
    au_init_from_mem(this, mem);
}

/// `realm::ArrayUnsigned::update_from_parent()`
#[export_name = "_ZN5realm13ArrayUnsigned18update_from_parentEv"]
pub unsafe extern "C" fn array_unsigned_update_from_parent(this: *mut ArrayUnsigned) {
    let new_ref = get_ref_from_parent(this);
    // ArrayUnsigned::init_from_ref (array_unsigned.hpp:39), inlined here as the C++
    // inlines it: translate the ref, then init_from_mem.
    let header = allocator_translate((*this).m_alloc, new_ref);
    au_init_from_mem(
        this,
        MemRef {
            addr: header,
            ref_: new_ref,
        },
    );
}

/// `realm::ArrayUnsigned::lower_bound(uint64_t) const`
#[export_name = "_ZNK5realm13ArrayUnsigned11lower_boundEy"]
pub unsafe extern "C" fn array_unsigned_lower_bound(this: *const ArrayUnsigned, value: u64) -> usize {
    let width = (*this).m_width;
    let data = (*this).m_data;
    let size = (*this).m_size;
    if width == 8 {
        std_lower_bound(data, size, value, 1)
    } else if width == 16 {
        std_lower_bound(data, size, value, 2)
    } else if width == 32 {
        std_lower_bound(data, size, value, 4)
    } else if width < 8 {
        match width {
            0 | 1 | 2 | 4 => lower_bound_direct(data, size, value as i64, width as usize),
            // REALM_UNREACHABLE() at array_unsigned.cpp:120 — live in this build.
            _ => unreachable(THIS_FILE, 120),
        }
    } else {
        std_lower_bound(data, size, value, 8)
    }
}

/// `realm::ArrayUnsigned::upper_bound(uint64_t) const`
#[export_name = "_ZNK5realm13ArrayUnsigned11upper_boundEy"]
pub unsafe extern "C" fn array_unsigned_upper_bound(this: *const ArrayUnsigned, value: u64) -> usize {
    let width = (*this).m_width;
    let data = (*this).m_data;
    let size = (*this).m_size;
    if width == 8 {
        std_upper_bound(data, size, value, 1)
    } else if width == 16 {
        std_upper_bound(data, size, value, 2)
    } else if width == 32 {
        std_upper_bound(data, size, value, 4)
    } else if width < 8 {
        match width {
            0 | 1 | 2 | 4 => upper_bound_direct(data, size, value as i64, width as usize),
            // REALM_UNREACHABLE() at array_unsigned.cpp:158.
            _ => unreachable(THIS_FILE, 158),
        }
    } else {
        std_upper_bound(data, size, value, 8)
    }
}

/// `realm::ArrayUnsigned::insert(size_t, uint64_t)`
#[export_name = "_ZN5realm13ArrayUnsigned6insertEmy"]
pub unsafe extern "C-unwind" fn array_unsigned_insert(
    this: *mut ArrayUnsigned,
    ndx: usize,
    value: u64,
) {
    let do_expand = value > (*this).m_ubound;
    let old_width = (*this).m_width;
    let new_width = if do_expand { bit_width(value) } else { (*this).m_width };
    let old_size = (*this).m_size;

    copy_on_write(this);
    au_alloc(this, (*this).m_size + 1, new_width as usize);

    if do_expand {
        // Move values above the insertion point, widening as we go. Downwards from
        // the top so a wider write never clobbers a not-yet-read narrower element.
        let mut i = old_size;
        while i > ndx {
            i -= 1;
            let tmp = _get(this, i, old_width);
            _set(this, i + 1, new_width, tmp);
        }
    } else if ndx != (*this).m_size {
        // NOTE: m_size here is the *new* size — Node::alloc already bumped it. The
        // C++ reads m_size after alloc too, so this compares ndx against old_size+1
        // and is therefore always true. Mirrored rather than simplified: turning it
        // into `ndx != old_size` would skip the (empty) copy on append, which is the
        // same bytes today but not the same code.
        let w = (new_width >> 3) as usize;
        // std::copy_backward over [ndx, old_size) to end at old_size+1.
        core::ptr::copy(
            (*this).m_data.add(ndx * w),
            (*this).m_data.add((ndx + 1) * w),
            (old_size - ndx) * w,
        );
    }

    _set(this, ndx, new_width, value);

    if do_expand {
        // Widen everything below the insertion point, again downwards.
        let mut i = ndx;
        while i != 0 {
            i -= 1;
            let v = _get(this, i, old_width);
            _set(this, i, new_width, v);
        }
    }
}

/// `realm::ArrayUnsigned::erase(size_t)`
#[export_name = "_ZN5realm13ArrayUnsigned5eraseEm"]
pub unsafe extern "C-unwind" fn array_unsigned_erase(this: *mut ArrayUnsigned, ndx: usize) {
    copy_on_write(this);

    let w = ((*this).m_width >> 3) as usize;
    let dst = (*this).m_data.add(ndx * w);
    let src = dst.add(w);
    let num_bytes = ((*this).m_size - ndx - 1) * w;

    // std::copy_n with dst < src; `ptr::copy` is memmove, which is correct for the
    // forward-overlapping case the C++ relies on.
    core::ptr::copy(src, dst, num_bytes);

    (*this).m_size -= 1;
    set_size_in_header((*this).m_size, get_header(this));
}

/// `realm::ArrayUnsigned::get(size_t) const`
#[export_name = "_ZNK5realm13ArrayUnsigned3getEm"]
pub unsafe extern "C" fn array_unsigned_get(this: *const ArrayUnsigned, index: usize) -> u64 {
    _get(this, index, (*this).m_width)
}

/// `realm::ArrayUnsigned::set(size_t, uint64_t)`
#[export_name = "_ZN5realm13ArrayUnsigned3setEmy"]
pub unsafe extern "C-unwind" fn array_unsigned_set(
    this: *mut ArrayUnsigned,
    ndx: usize,
    value: u64,
) {
    copy_on_write(this);

    if value > (*this).m_ubound {
        let old_width = (*this).m_width;
        let new_width = bit_width(value);

        au_alloc(this, (*this).m_size, new_width as usize);

        let mut i = (*this).m_size;
        while i != 0 {
            i -= 1;
            let v = _get(this, i, old_width);
            _set(this, i, new_width, v);
        }
    }

    // m_width, not new_width: au_alloc has already updated it, and on the
    // non-expanding path there is no new_width at all.
    _set(this, ndx, (*this).m_width, value);
}

/// `realm::ArrayUnsigned::truncate(size_t)`
#[export_name = "_ZN5realm13ArrayUnsigned8truncateEm"]
pub unsafe extern "C-unwind" fn array_unsigned_truncate(this: *mut ArrayUnsigned, ndx: usize) {
    // m_size is assigned *before* copy_on_write, so the copy sees the truncated size.
    // Order matters here: it decides how much do_copy_on_write moves.
    (*this).m_size = ndx;
    copy_on_write(this);
    set_size_in_header((*this).m_size, get_header(this));
    if ndx == 0 {
        set_width_impl(this, 8);
        set_width_in_header(8, get_header(this));
    }
}
