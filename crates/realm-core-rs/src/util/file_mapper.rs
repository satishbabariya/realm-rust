//! `upstream/src/realm/util/file_mapper.cpp` — the memory-mapping primitives.
//!
//! **Byte-visible and traced.** Every trace maps and syncs, so `make diff-test` judges
//! this directly rather than through a differential. Only `array_unsigned` has been in
//! that category before.
//!
//! # Exceptions: all three kinds appear here
//!
//! Settled by `migration/checks/throw_probe/` before any of this was written:
//!
//! - **throwing** — four types, every constructor bound rather than reproduced.
//!   `AddressSpaceExhausted`'s constructor is inline, so it is built by running the
//!   bindable `RuntimeError` base constructor and overwriting the vptr; the probe checks
//!   that with `typeid`, because a derived-type `catch` matches on the tag passed to
//!   `__cxa_throw` and cannot see a wrong vptr.
//! - **cleanup while an exception passes through** — `mmap`'s `ScopeExitFail` becomes a
//!   Rust `Drop`. The probe confirms drops run for a foreign unwind, exactly once, and
//!   not on the success path.
//! - **catching** — does not appear here, which is what makes the unit portable at all.
//!
//! # Measured
//!
//! ```text
//! FileAttributes  sizeof 16   fd i32 @0, access i32 @4, encryption ptr @8
//! File::AccessMode            access_ReadOnly = 0, access_ReadWrite = 1
//! unique_ptr<EncryptedFileMapping>   8 bytes, but a user-provided destructor makes it
//!                                    MEMORY class -> returned via sret, never in rax.
//!                                    Same trap as `bind_ptr` in status.cpp.
//! ```
//!
//! # A note on message strings
//!
//! The error messages are built to match the C++ text exactly, but their `std::string`
//! *capacity* is not reproduced with the care `decimal128::to_string` needed. These
//! strings are exception payloads on failure paths — they never reach a `.realm`, and no
//! caller inspects their capacity. Where capacity is observable (a value-returning API)
//! it is matched; here it is not.

use core::ffi::{c_char, c_int, c_uint, c_void};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// `realm::util::FileAttributes`
#[repr(C)]
pub struct FileAttributes {
    fd: c_int,
    access: c_int,
    encryption: *mut c_void, // EncryptedFile*
}

const ACCESS_READ_ONLY: c_int = 0;
const ACCESS_READ_WRITE: c_int = 1;

/// libc++ `std::string`, 24 bytes.
#[repr(C, align(8))]
struct StdString {
    rep: [u8; 24],
}

/// `realm::util::Printable` — `Type` at 0, four bytes of padding, union at 8.
#[repr(C)]
struct Printable {
    ty: u32,
    _pad: u32,
    val: [usize; 2],
}
const PRINTABLE_UINT: u32 = 2;
const PRINTABLE_STRING: u32 = 4;

/// `std::initializer_list<Printable>` — `{const Printable*, size_t}`.
#[repr(C)]
struct InitializerList {
    begin: *const Printable,
    size: usize,
}

const _: () = {
    assert!(core::mem::size_of::<FileAttributes>() == 16);
    assert!(core::mem::size_of::<StdString>() == 24);
    assert!(core::mem::size_of::<Printable>() == 24);
};

// ---------------------------------------------------------------------------
// Bound symbols
// ---------------------------------------------------------------------------

extern "C" {
    fn mmap(addr: *mut c_void, len: usize, prot: c_int, flags: c_int, fd: c_int, offset: i64) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> c_int;
    fn msync(addr: *mut c_void, len: usize, flags: c_int) -> c_int;
    #[link_name = "__error"]
    fn errno_location() -> *mut c_int;

    #[link_name = "_ZN5realm4util9page_sizeEv"]
    fn page_size() -> usize;

    /// `realm::util::format(const char*, std::initializer_list<Printable>)` — sret.
    #[link_name = "_ZN5realm4util6formatEPKcSt16initializer_listINS0_9PrintableEE"]
    fn realm_format(sret: *mut StdString, fmt: *const c_char, values: InitializerList);

    /// `realm::util::error::make_error_code(basic_system_errors)` -> `std::error_code`.
    ///
    /// `util::make_basic_system_error_code(int)` is **inline** (basic_system_errors.hpp:69)
    /// and forwards to this, so this is the symbol to bind. `std::error_code` is 16 bytes
    /// and trivially copyable, so it comes back in registers rather than through `sret`.
    #[link_name = "_ZN5realm4util5error15make_error_codeENS1_19basic_system_errorsE"]
    fn make_basic_system_error_code(err: c_int) -> ErrorCode;

    /// `std::error_code::message() const` — sret.
    #[link_name = "_ZNKSt3__110error_code7messageEv"]
    fn error_code_message(sret: *mut StdString, this: *const ErrorCode);

    /// `realm::util::Printable::str() const` — used by `util::to_string`.
    #[link_name = "_ZNK5realm4util9Printable3strEv"]
    fn printable_str(sret: *mut StdString, this: *const Printable);

    #[link_name = "_ZNSt3__112basic_stringIcNS_11char_traitsIcEENS_9allocatorIcEEE6appendEPKcm"]
    fn string_append(this: *mut StdString, s: *const c_char, n: usize) -> *mut StdString;
    #[link_name = "_ZNSt3__112basic_stringIcNS_11char_traitsIcEENS_9allocatorIcEEED1Ev"]
    fn string_dtor(this: *mut StdString);

    /// `EncryptedFile::add_mapping(File::SizeType, void*, size_t, File::AccessMode)`
    /// returns `unique_ptr<EncryptedFileMapping>` by value — **sret**.
    #[link_name = "_ZN5realm4util13EncryptedFile11add_mappingExPvmNS0_4File10AccessModeE"]
    fn encrypted_file_add_mapping(
        sret: *mut *mut c_void,
        this: *mut c_void,
        file_offset: i64,
        addr: *mut c_void,
        size: usize,
        access: c_int,
    );

    #[link_name = "_ZN5realm4util20EncryptedFileMapping12read_barrierEPKvmb"]
    fn encrypted_mapping_read_barrier(this: *mut c_void, addr: *const c_void, size: usize, to_modify: bool);
    #[link_name = "_ZN5realm4util20EncryptedFileMapping13write_barrierEPKvm"]
    fn encrypted_mapping_write_barrier(this: *mut c_void, addr: *const c_void, size: usize);
    #[link_name = "_ZN5realm4util20EncryptedFileMappingD1Ev"]
    fn encrypted_mapping_dtor(this: *mut c_void);

    #[link_name = "_ZdlPv"]
    fn cxx_operator_delete(p: *mut c_void);
}

/// `std::error_code` — `{int val, const error_category* cat}`.
#[repr(C)]
struct ErrorCode {
    val: c_int,
    _pad: c_int,
    cat: *const c_void,
}

// libc constants, from <sys/mman.h> on Darwin.
const PROT_READ: c_int = 0x01;
const PROT_WRITE: c_int = 0x02;
const MAP_SHARED: c_int = 0x0001;
const MAP_PRIVATE: c_int = 0x0002;
const MAP_FIXED: c_int = 0x0010;
const MAP_ANON: c_int = 0x1000;
const MS_SYNC: c_int = 0x0010;
const MAP_FAILED: *mut c_void = usize::MAX as *mut c_void;

const EINTR: c_int = 4;
const EAGAIN: c_int = 35;
const ENOMEM: c_int = 12;
const EMFILE: c_int = 24;

/// `_impl::SimulatedFailure::trigger_mmap(size)` — **a no-op in this build**, so it is
/// omitted rather than bound. `REALM_ENABLE_SIMULATED_FAILURE` is defined only under
/// `REALM_DEBUG` (`simulated_failure.hpp:28`), which is off here, so the inline body is
/// `static_cast<void>(size)` and no symbol exists to call. Confirmed by the link failing
/// on it when it *was* bound.
#[inline]
fn trigger_mmap(_size: usize) {}

/// `is_mmap_memory_error` — the anonymous-namespace helper at the top of the unit.
#[inline]
fn is_mmap_memory_error(err: c_int) -> bool {
    err == EAGAIN || err == EMFILE || err == ENOMEM
}

#[inline]
unsafe fn errno() -> c_int {
    *errno_location()
}

// ---------------------------------------------------------------------------
// String helpers
// ---------------------------------------------------------------------------

impl StdString {
    const EMPTY: StdString = StdString { rep: [0u8; 24] };

    #[inline]
    unsafe fn parts(&self) -> (*const c_char, usize) {
        let base = self.rep.as_ptr();
        if base.read() & 1 == 0 {
            (base.add(1) as *const c_char, (base.read() >> 1) as usize)
        } else {
            let w = base as *const usize;
            (w.add(2).read_unaligned() as *const c_char, w.add(1).read_unaligned())
        }
    }
}

/// `util::to_string(v)` == `Printable(v).str()`.
#[inline]
unsafe fn to_string_uint(v: usize) -> StdString {
    let p = Printable { ty: PRINTABLE_UINT, _pad: 0, val: [v, 0] };
    let mut out = core::mem::MaybeUninit::<StdString>::uninit();
    printable_str(out.as_mut_ptr(), &p);
    out.assume_init()
}

/// `get_errno_msg(prefix, err)` — `prefix + make_basic_system_error_code(err).message()`.
#[inline]
unsafe fn get_errno_msg(prefix: &str, err: c_int) -> StdString {
    let code = make_basic_system_error_code(err);
    let mut msg = core::mem::MaybeUninit::<StdString>::uninit();
    error_code_message(msg.as_mut_ptr(), &code);
    let msg = msg.assume_init();

    // `const char* + std::string` builds a fresh string: prefix then the message.
    let mut out = StdString::EMPTY;
    string_append(&mut out, prefix.as_ptr() as *const c_char, prefix.len());
    let (p, n) = msg.parts();
    string_append(&mut out, p, n);
    let mut msg = msg;
    string_dtor(&mut msg);
    out
}

#[inline]
unsafe fn string_push_str(s: *mut StdString, text: &str) {
    string_append(s, text.as_ptr() as *const c_char, text.len());
}

#[inline]
unsafe fn string_push_string(s: *mut StdString, other: &StdString) {
    let (p, n) = other.parts();
    string_append(s, p, n);
}

// ---------------------------------------------------------------------------
// Throwing. See migration/checks/throw_probe/ — every constructor is bound.
// ---------------------------------------------------------------------------

mod throwing {
    use super::{StdString, c_char, c_int, c_void};

    #[repr(C)]
    pub(super) struct StringView {
        pub data: *const c_char,
        pub size: usize,
    }

    extern "C" {
        pub(super) fn __cxa_allocate_exception(size: usize) -> *mut c_void;

        #[link_name = "_ZN5realm12RuntimeErrorC1ENS_10ErrorCodes5ErrorENSt3__117basic_string_viewIcNS3_11char_traitsIcEEEE"]
        pub(super) fn runtime_error_ctor(this: *mut c_void, code: c_int, msg: StringView);
        #[link_name = "_ZN5realm21AddressSpaceExhaustedD1Ev"]
        pub(super) fn ase_dtor(this: *mut c_void);
        #[link_name = "_ZTVN5realm21AddressSpaceExhaustedE"]
        pub(super) static ASE_VTABLE: c_void;
        #[link_name = "_ZTIN5realm21AddressSpaceExhaustedE"]
        pub(super) static ASE_TYPEINFO: c_void;

        #[link_name = "_ZN5realm11SystemErrorC1EiNSt3__117basic_string_viewIcNS1_11char_traitsIcEEEE"]
        pub(super) fn realm_system_error_ctor(this: *mut c_void, err: c_int, msg: StringView);
        #[link_name = "_ZN5realm11SystemErrorD1Ev"]
        pub(super) fn realm_system_error_dtor(this: *mut c_void);
        #[link_name = "_ZTIN5realm11SystemErrorE"]
        pub(super) static REALM_SYSTEM_ERROR_TYPEINFO: c_void;

        #[link_name = "_ZNSt3__112system_errorC1EiRKNS_14error_categoryERKNS_12basic_stringIcNS_11char_traitsIcEENS_9allocatorIcEEEE"]
        pub(super) fn std_system_error_ctor(this: *mut c_void, ev: c_int, cat: *const c_void, what: *const StdString);
        #[link_name = "_ZNSt3__112system_errorD1Ev"]
        pub(super) fn std_system_error_dtor(this: *mut c_void);
        #[link_name = "_ZTINSt3__112system_errorE"]
        pub(super) static STD_SYSTEM_ERROR_TYPEINFO: c_void;
        #[link_name = "_ZNSt3__115system_categoryEv"]
        pub(super) fn std_system_category() -> *const c_void;

        #[link_name = "_ZNSt13runtime_errorC1ERKNSt3__112basic_stringIcNS0_11char_traitsIcEENS0_9allocatorIcEEEE"]
        pub(super) fn std_runtime_error_ctor(this: *mut c_void, what: *const StdString);
        #[link_name = "_ZNSt13runtime_errorD1Ev"]
        pub(super) fn std_runtime_error_dtor(this: *mut c_void);
        #[link_name = "_ZTISt13runtime_error"]
        pub(super) static STD_RUNTIME_ERROR_TYPEINFO: c_void;
    }

    extern "C-unwind" {
        pub(super) fn __cxa_throw(
            exc: *mut c_void,
            tinfo: *const c_void,
            dest: unsafe extern "C" fn(*mut c_void),
        ) -> !;
    }
}

const ERRORCODES_ADDRESS_SPACE_EXHAUSTED: c_int = 1005;

/// `throw AddressSpaceExhausted(msg)`. Its constructor is inline, so the object is built
/// through the `RuntimeError` base and the vptr is corrected afterwards.
#[inline(never)]
unsafe fn throw_address_space_exhausted(msg: &StdString) -> ! {
    use throwing::*;
    let (p, n) = msg.parts();
    let e = __cxa_allocate_exception(16);
    runtime_error_ctor(
        e,
        ERRORCODES_ADDRESS_SPACE_EXHAUSTED,
        StringView { data: p, size: n },
    );
    *(e as *mut *const u8) = (&ASE_VTABLE as *const c_void as *const u8).add(16);
    __cxa_throw(e, &ASE_TYPEINFO as *const c_void, ase_dtor)
}

/// `throw realm::SystemError(err, msg)`.
#[inline(never)]
unsafe fn throw_realm_system_error(err: c_int, msg: &StdString) -> ! {
    use throwing::*;
    let (p, n) = msg.parts();
    let e = __cxa_allocate_exception(16);
    realm_system_error_ctor(e, err, StringView { data: p, size: n });
    __cxa_throw(e, &REALM_SYSTEM_ERROR_TYPEINFO as *const c_void, realm_system_error_dtor)
}

/// `throw std::system_error(err, std::system_category(), what)`.
#[inline(never)]
unsafe fn throw_std_system_error(err: c_int, what: &StdString) -> ! {
    use throwing::*;
    let e = __cxa_allocate_exception(32);
    std_system_error_ctor(e, err, std_system_category(), what);
    __cxa_throw(e, &STD_SYSTEM_ERROR_TYPEINFO as *const c_void, std_system_error_dtor)
}

/// `throw std::runtime_error(what)`.
#[inline(never)]
unsafe fn throw_std_runtime_error(what: &StdString) -> ! {
    use throwing::*;
    let e = __cxa_allocate_exception(16);
    std_runtime_error_ctor(e, what);
    __cxa_throw(e, &STD_RUNTIME_ERROR_TYPEINFO as *const c_void, std_runtime_error_dtor)
}

/// `ScopeExitFail` — runs its action only when the scope is left by an exception.
///
/// A Rust `Drop` runs during a foreign unwind (verified in
/// `migration/checks/throw_probe/drop_probe.rs`), and `disarm()` before the normal return
/// gives the "on failure only" half. This is what replaces a `catch` Rust does not have.
struct ScopeExitFail {
    addr: *mut c_void,
    size: usize,
    armed: bool,
}

impl ScopeExitFail {
    #[inline]
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ScopeExitFail {
    fn drop(&mut self) {
        if self.armed {
            unsafe { _ZN5realm4util6munmapEPvm(self.addr, self.size) };
        }
    }
}

// ---------------------------------------------------------------------------
// Exported surface
// ---------------------------------------------------------------------------

/// `realm::util::round_up_to_page_size(size_t) noexcept`
#[no_mangle]
pub unsafe extern "C" fn _ZN5realm4util21round_up_to_page_sizeEm(size: usize) -> usize {
    let ps = page_size();
    (size.wrapping_add(ps).wrapping_sub(1)) & !(ps.wrapping_sub(1))
}

/// `realm::util::mmap(const FileAttributes&, size_t, uint64_t, unique_ptr<EncryptedFileMapping>&)`
#[no_mangle]
pub unsafe extern "C-unwind" fn _ZN5realm4util4mmapERKNS0_14FileAttributesEmyRNSt3__110unique_ptrINS0_20EncryptedFileMappingENS4_14default_deleteIS6_EEEE(
    file: *const FileAttributes,
    mut size: usize,
    offset: u64,
    mapping: *mut *mut c_void,
) -> *mut c_void {
    trigger_mmap(size);

    if !(*file).encryption.is_null() {
        let ps = page_size();
        let page_start = offset & !((ps as u64) - 1);
        size += (offset - page_start) as usize;
        size = _ZN5realm4util21round_up_to_page_sizeEm(size);
        let addr = _ZN5realm4util9mmap_anonEm(size);

        let mut cleanup = ScopeExitFail { addr, size, armed: true };
        let mut fresh: *mut c_void = core::ptr::null_mut();
        encrypted_file_add_mapping(
            &mut fresh,
            (*file).encryption,
            page_start as i64,
            addr,
            size,
            (*file).access,
        ); // Throws — the guard above munmaps if it does
        cleanup.disarm();

        // `mapping = <temporary>` is unique_ptr's move-assign: destroy the old target,
        // take the new pointer, leave the temporary null.
        let old = *mapping;
        *mapping = fresh;
        if !old.is_null() {
            encrypted_mapping_dtor(old);
            cxx_operator_delete(old);
        }

        return (addr as *mut u8).offset(-(page_start as isize)).add(offset as usize) as *mut c_void;
    }

    // `mapping = nullptr` — also a move-assign from nullptr_t, so the old target dies.
    let old = *mapping;
    *mapping = core::ptr::null_mut();
    if !old.is_null() {
        encrypted_mapping_dtor(old);
        cxx_operator_delete(old);
    }

    let mut prot = PROT_READ;
    if (*file).access == ACCESS_READ_WRITE {
        prot |= PROT_WRITE;
    }

    let addr = mmap(
        core::ptr::null_mut(),
        size,
        prot,
        MAP_SHARED,
        (*file).fd,
        offset as i64,
    );
    if addr != MAP_FAILED {
        return addr;
    }

    let err = errno(); // Eliminate any risk of clobbering
    if is_mmap_memory_error(err) {
        let code = make_basic_system_error_code(err);
        let mut m = core::mem::MaybeUninit::<StdString>::uninit();
        error_code_message(m.as_mut_ptr(), &code);
        let m = m.assume_init();
        let (mp, mn) = m.parts();
        let vals = [
            Printable { ty: PRINTABLE_STRING, _pad: 0, val: [mp as usize, mn] },
            Printable { ty: PRINTABLE_UINT, _pad: 0, val: [size, 0] },
            Printable { ty: PRINTABLE_UINT, _pad: 0, val: [offset as usize, 0] },
        ];
        let mut msg = core::mem::MaybeUninit::<StdString>::uninit();
        realm_format(
            msg.as_mut_ptr(),
            b"mmap() failed: %1 (size: %2, offset: %3)\0".as_ptr() as *const c_char,
            InitializerList { begin: vals.as_ptr(), size: 3 },
        );
        let msg = msg.assume_init();
        let mut m = m;
        string_dtor(&mut m);
        throw_address_space_exhausted(&msg);
    }

    let vals = [
        Printable { ty: PRINTABLE_UINT, _pad: 0, val: [size, 0] },
        Printable { ty: PRINTABLE_UINT, _pad: 0, val: [offset as usize, 0] },
    ];
    let mut msg = core::mem::MaybeUninit::<StdString>::uninit();
    realm_format(
        msg.as_mut_ptr(),
        b"mmap() failed (size: %1, offset: %2\0".as_ptr() as *const c_char,
        InitializerList { begin: vals.as_ptr(), size: 2 },
    );
    let msg = msg.assume_init();
    throw_realm_system_error(err, &msg);
}

/// `realm::util::mmap_anon(size_t)`
#[no_mangle]
pub unsafe extern "C-unwind" fn _ZN5realm4util9mmap_anonEm(size: usize) -> *mut c_void {
    let addr = mmap(
        core::ptr::null_mut(),
        size,
        PROT_READ | PROT_WRITE,
        MAP_ANON | MAP_PRIVATE,
        -1,
        0,
    );
    if addr == MAP_FAILED {
        let err = errno(); // Eliminate any risk of clobbering
        if is_mmap_memory_error(err) {
            // get_errno_msg("mmap() failed: ", err) + " size: " + util::to_string(size)
            let mut msg = get_errno_msg("mmap() failed: ", err);
            string_push_str(&mut msg, " size: ");
            let n = to_string_uint(size);
            string_push_string(&mut msg, &n);
            let mut n = n;
            string_dtor(&mut n);
            throw_address_space_exhausted(&msg);
        }
        // std::string("mmap() failed (size: ") + util::to_string(size) + ", offset is 0)"
        let mut what = StdString::EMPTY;
        string_push_str(&mut what, "mmap() failed (size: ");
        let n = to_string_uint(size);
        string_push_string(&mut what, &n);
        let mut n = n;
        string_dtor(&mut n);
        string_push_str(&mut what, ", offset is 0)");
        throw_std_system_error(err, &what);
    }
    addr
}

/// `realm::util::mmap_fixed(FileDesc, void*, size_t, File::AccessMode, uint64_t)`
#[no_mangle]
pub unsafe extern "C-unwind" fn _ZN5realm4util10mmap_fixedEiPvmNS0_4File10AccessModeEy(
    fd: c_int,
    address_request: *mut c_void,
    size: usize,
    access: c_int,
    offset: u64,
) -> *mut c_void {
    trigger_mmap(size);
    let mut prot = PROT_READ;
    if access == ACCESS_READ_WRITE {
        prot |= PROT_WRITE;
    }
    let addr = mmap(
        address_request,
        size,
        prot,
        MAP_SHARED | MAP_FIXED,
        fd,
        offset as i64,
    );
    if addr != MAP_FAILED && addr != address_request {
        // Note the C++ reads `errno` here even though mmap succeeded; mirrored.
        let mut what = get_errno_msg("mmap() failed: ", errno());
        string_push_str(&mut what, ", when mapping an already reserved memory area");
        throw_std_runtime_error(&what);
    }
    addr
}

/// `realm::util::munmap(void*, size_t)`
#[no_mangle]
pub unsafe extern "C-unwind" fn _ZN5realm4util6munmapEPvm(addr: *mut c_void, size: usize) {
    let shift = (addr as usize) & (page_size() - 1);
    let addr = (addr as *mut u8).sub(shift) as *mut c_void;
    let size = size + shift;
    if munmap(addr, size) != 0 {
        let err = errno();
        let mut what = StdString::EMPTY;
        string_push_str(&mut what, "munmap() failed");
        throw_std_system_error(err, &what);
    }
}

/// `realm::util::msync(FileDesc, void*, size_t)`
#[no_mangle]
pub unsafe extern "C-unwind" fn _ZN5realm4util5msyncEiPvm(_fd: c_int, addr: *mut c_void, size: usize) {
    let mut retries_left: c_int = 1000;
    while msync(addr, size, MS_SYNC) != 0 {
        let err = errno(); // Eliminate any risk of clobbering
        retries_left -= 1;
        if retries_left < 0 {
            let mut what = StdString::EMPTY;
            string_push_str(&mut what, "msync() retries exhausted");
            throw_std_system_error(err, &what);
        }
        if err != EINTR {
            let mut what = StdString::EMPTY;
            string_push_str(&mut what, "msync() failed");
            throw_std_system_error(err, &what);
        }
    }
}

/// `realm::util::reserve_mapping(void*, const FileAttributes&, uint64_t)` — returns
/// `unique_ptr` by value, which is MEMORY class, so **sret**.
#[no_mangle]
pub unsafe extern "C-unwind" fn _ZN5realm4util15reserve_mappingEPvRKNS0_14FileAttributesEy(
    sret: *mut *mut c_void,
    addr: *mut c_void,
    file: *const FileAttributes,
    offset: u64,
) -> *mut *mut c_void {
    encrypted_file_add_mapping(sret, (*file).encryption, offset as i64, addr, 0, (*file).access);
    sret
}

/// `realm::util::do_encryption_read_barrier(const void*, size_t, EncryptedFileMapping*, bool)`
#[no_mangle]
pub unsafe extern "C-unwind" fn _ZN5realm4util26do_encryption_read_barrierEPKvmPNS0_20EncryptedFileMappingEb(
    addr: *const c_void,
    size: usize,
    mapping: *mut c_void,
    to_modify: bool,
) {
    encrypted_mapping_read_barrier(mapping, addr, size, to_modify);
}

/// `realm::util::do_encryption_write_barrier(const void*, size_t, EncryptedFileMapping*)`
#[no_mangle]
pub unsafe extern "C-unwind" fn _ZN5realm4util27do_encryption_write_barrierEPKvmPNS0_20EncryptedFileMappingE(
    addr: *const c_void,
    size: usize,
    mapping: *mut c_void,
) {
    encrypted_mapping_write_barrier(mapping, addr, size);
}
