//! Shared mimalloc tuning for the UI and daemon binaries.
//!
//! Both binaries install `mimalloc::MiMalloc` as the global allocator. Its
//! defaults keep freed pages around for a while (the purge delay is 1s for
//! thread heaps and multiplied for arenas), which leaves the process RSS near
//! the peak after a burst of decoding or compression. The UI in particular
//! keeps a decoded-image cache, so it is better to return freed pages to the OS
//! promptly even at the cost of a few more `madvise` calls.
//!
//! `libmimalloc-sys` 0.1 compiles mimalloc v3 but only re-exports the v2 option
//! constants. The v3-only ids below come from `mimalloc.h` in that release and
//! must be checked if the `mimalloc` dependency moves past 0.1.

use libmimalloc_sys::mi_option_set;

/// `mi_option_purge_decommits`: decommit pages on purge (Linux uses
/// `MADV_DONTNEED`) instead of merely resetting them.
const MI_OPTION_PURGE_DECOMMITS: i32 = 5;
/// `mi_option_purge_delay`: milliseconds before a freed page is purged. 0 is
/// immediate; -1 disables purging.
const MI_OPTION_PURGE_DELAY: i32 = 15;
/// `mi_option_arena_purge_mult`: purge-delay multiplier for arenas.
const MI_OPTION_ARENA_PURGE_MULT: i32 = 24;
/// `mi_option_page_reclaim_on_free`: reclaim abandoned pages even when they did
/// not originate from the current thread heap.
const MI_OPTION_PAGE_RECLAIM_ON_FREE: i32 = 35;
/// `mi_option_page_full_retain`: full pages retained per size class.
const MI_OPTION_PAGE_FULL_RETAIN: i32 = 36;

/// Makes freed mimalloc pages return to the OS early.
///
/// # Safety
///
/// This mutates mimalloc's process-global option table. Call it once, first
/// thing in `main`, before worker threads start allocating.
pub fn configure_allocator() {
    // SAFETY: mimalloc documents the option API as not thread safe, and the
    // binaries call this before spawning any thread.
    unsafe {
        mi_option_set(MI_OPTION_PURGE_DECOMMITS, 1);
        mi_option_set(MI_OPTION_PURGE_DELAY, 0);
        mi_option_set(MI_OPTION_ARENA_PURGE_MULT, 1);
        mi_option_set(MI_OPTION_PAGE_RECLAIM_ON_FREE, 1);
        mi_option_set(MI_OPTION_PAGE_FULL_RETAIN, 0);
    }
}
