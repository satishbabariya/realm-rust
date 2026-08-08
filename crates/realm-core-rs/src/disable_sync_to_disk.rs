//! Port of `upstream/src/realm/disable_sync_to_disk.cpp`.
//!
//! A process-wide flag that suppresses `fsync`/`msync` during writes. Used by the
//! test suite to make runs faster; production code leaves it false.
//!
//! ## What the byte-identity gate can and cannot prove here
//!
//! Nothing in this unit reaches the file. All five call sites use the flag only to
//! decide whether to *flush*, never what to write:
//!
//! ```text
//! db.cpp:1680           bool disable_sync = get_disable_sync_to_disk();
//! alloc_slab.cpp:839    bool disable_sync = get_disable_sync_to_disk() || cfg.disable_sync;
//! alloc_slab.cpp:1509   bool disable_sync = get_disable_sync_to_disk() || m_cfg.disable_sync;
//! alloc_slab.cpp:1532   bool disable_sync = get_disable_sync_to_disk() || m_cfg.disable_sync;
//! group_writer.cpp:1402 bool disable_sync = get_disable_sync_to_disk() || m_durability == Durability::Unsafe;
//! ```
//!
//! So an implementation that returned the *wrong* value would still produce a
//! byte-identical `.realm`, and `make diff-test` would pass. The gate confirms this
//! port breaks nothing; it does not confirm the port is correct. That is why the
//! unit tests below check the observable contract directly, in the spirit of
//! `migration/checks/base64_differential.cpp`.
//!
//! See `migration/JOURNAL.md` for the reachability analysis that put this unit ahead
//! of the six dead units at the head of the queue.

use std::sync::atomic::{AtomicBool, Ordering};

/// Mirror of the anonymous-namespace global in the C++ TU:
///
/// ```cpp
/// std::atomic<bool> g_disable_sync_to_disk(false);
/// ```
///
/// The C++ object has internal linkage, so it exports no symbol and there is exactly
/// one instance of this flag once the Rust definitions win the link.
static G_DISABLE_SYNC_TO_DISK: AtomicBool = AtomicBool::new(false);

/// `realm::disable_sync_to_disk(bool)`
///
/// C++ body is `g_disable_sync_to_disk = disable;`. `std::atomic`'s assignment
/// operator is `store(desired, memory_order_seq_cst)`, so this is **SeqCst and not
/// Relaxed**. Relaxed would almost certainly behave the same on every platform realm
/// targets, which is exactly why it is worth stating: mirroring the C++ is the rule,
/// and "this ordering looked stronger than necessary" is not a reason to weaken it.
#[export_name = "_ZN5realm20disable_sync_to_diskEb"]
pub extern "C" fn disable_sync_to_disk(disable: bool) {
    G_DISABLE_SYNC_TO_DISK.store(disable, Ordering::SeqCst);
}

/// `realm::get_disable_sync_to_disk()`
///
/// C++ body is `return g_disable_sync_to_disk;`. The implicit conversion to `bool` is
/// `load(memory_order_seq_cst)` — same reasoning as the setter.
///
/// Declared `noexcept` in C++; `extern "C"` in Rust already forbids unwinding across
/// the boundary, and nothing here can panic.
#[export_name = "_ZN5realm24get_disable_sync_to_diskEv"]
pub extern "C" fn get_disable_sync_to_disk() -> bool {
    G_DISABLE_SYNC_TO_DISK.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The flag is process-global, so the tests that mutate it must not interleave.
    static SERIALISE: Mutex<()> = Mutex::new(());

    #[test]
    fn defaults_to_false() {
        let _g = SERIALISE.lock().unwrap();
        // Only meaningful before anything sets it, but the round-trip test restores
        // false, so this holds regardless of test order.
        assert!(!get_disable_sync_to_disk());
    }

    #[test]
    fn round_trips_both_ways() {
        let _g = SERIALISE.lock().unwrap();
        disable_sync_to_disk(true);
        assert!(get_disable_sync_to_disk());
        disable_sync_to_disk(false);
        assert!(!get_disable_sync_to_disk());
    }

    /// The setter is a plain store, not a toggle or a latch — setting the same value
    /// twice is idempotent, and setting false after true genuinely clears it.
    #[test]
    fn is_a_store_not_a_latch() {
        let _g = SERIALISE.lock().unwrap();
        disable_sync_to_disk(true);
        disable_sync_to_disk(true);
        assert!(get_disable_sync_to_disk());
        disable_sync_to_disk(false);
        assert!(!get_disable_sync_to_disk());
    }
}
