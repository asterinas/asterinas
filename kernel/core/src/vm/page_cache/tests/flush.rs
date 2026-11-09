// SPDX-License-Identifier: MPL-2.0

//! Tests for writing cached data to the backend with `PageCache::flush_range`.
//!
//! These cover pages changed during a flush and flushes running at the same time.

use alloc::vec;

use ostd::{mm::VmIo, prelude::ktest};

use super::base::{self, IoCompletion, IoKind, MockPageCacheBackend};
use crate::{
    prelude::*,
    thread::kernel_thread::ThreadOptions,
    vm::page_cache::{CachePageExt, VmoMapMode},
};

/// Checks that finishing a write leaves any newer changes to the page dirty.
/// The next flush must write those changes to the backend.
#[ktest]
fn preserves_redirty() {
    let backend = MockPageCacheBackend::new(1);
    backend.set_completion(IoKind::Write, IoCompletion::Deferred);
    let page_cache = base::new_locked_backend_page_cache(&backend, 1);
    let vmo = page_cache.read().as_vmo().clone();
    let first_pattern = vec![0x11; PAGE_SIZE];
    let latest_pattern = vec![0x33; PAGE_SIZE];
    page_cache.write().write_bytes(0, &first_pattern).unwrap();
    let (page, _) = vmo.try_commit_page(0, VmoMapMode::SharedRead).unwrap();

    let result = Arc::new(Mutex::new(None::<Result<()>>));
    let flush_thread = {
        let page_cache = page_cache.clone();
        let result = result.clone();
        ThreadOptions::new(move || {
            let page_cache = page_cache.read();
            *result.lock() = Some(page_cache.flush_range(0..PAGE_SIZE));
        })
        .spawn()
    };

    backend.wait_for_deferred_bios(IoKind::Write, 1);
    {
        let locked_page = page.lock_guard();
        assert!(locked_page.state().is_up_to_date());
        assert!(locked_page.is_writing_back());
    }
    // The flush is in flight; a mapped store bypasses its outer lock.
    base::write_first_page_via_vmo(&vmo, &latest_pattern);
    assert!(page.state().is_dirty());
    assert!(backend.complete_next_deferred_bio(IoKind::Write, true));
    flush_thread.join();

    assert!(result.lock().take().unwrap().is_ok());
    {
        let locked_page = page.lock_guard();
        assert!(locked_page.state().is_dirty());
        assert!(!locked_page.is_writing_back());
    }
    // Completing writeback must leave the newest cached bytes readable.
    let mut read_buffer = vec![0; PAGE_SIZE];
    page_cache.read().read_bytes(0, &mut read_buffer).unwrap();
    assert_eq!(read_buffer, latest_pattern);
    assert_eq!(backend.read_count(0), 0);
    assert_eq!(backend.write_count(0), 1);

    // The remaining dirty data is eligible for a later writeback.
    backend.set_completion(IoKind::Write, IoCompletion::Immediate);
    page_cache.read().flush_range(0..PAGE_SIZE).unwrap();
    assert_eq!(backend.write_count(0), 2);
    assert_eq!(backend.persisted_page_bytes(0), latest_pattern);
    {
        let locked_page = page.lock_guard();
        assert!(locked_page.state().is_up_to_date());
        assert!(!locked_page.is_writing_back());
    }
    page_cache.read().flush_range(0..PAGE_SIZE).unwrap();
    assert_eq!(backend.write_count(0), 2);
}

/// Checks that a second flush waits for the first write before writing the page again.
#[ktest]
fn serializes_flushes() {
    let backend = MockPageCacheBackend::new(1);
    backend.set_completion(IoKind::Write, IoCompletion::Deferred);
    let page_cache = base::new_locked_backend_page_cache(&backend, 1);
    let vmo = page_cache.read().as_vmo().clone();
    let first_dirty_pattern = vec![0x11; PAGE_SIZE];
    let latest_dirty_pattern = vec![0x33; PAGE_SIZE];

    // Start one deferred writeback, then dirty the page again before it
    // completes so a second flush must wait for `is_writing_back` to clear.
    page_cache
        .write()
        .write_bytes(0, &first_dirty_pattern)
        .unwrap();

    let first_flush_result = Arc::new(Mutex::new(None::<Result<()>>));
    let second_flush_result = Arc::new(Mutex::new(None::<Result<()>>));
    let second_flush_finished = Arc::new(Mutex::new(false));

    let first_flush_thread = {
        let page_cache = page_cache.clone();
        let first_flush_result = first_flush_result.clone();
        ThreadOptions::new(move || {
            *first_flush_result.lock() = Some(page_cache.read().flush_range(0..PAGE_SIZE));
        })
        .spawn()
    };

    backend.wait_for_deferred_bios(IoKind::Write, 1);

    base::write_first_page_via_vmo(&vmo, &latest_dirty_pattern);
    let (page, _) = vmo.try_commit_page(0, VmoMapMode::SharedRead).unwrap();

    let second_flush_thread = {
        let page_cache = page_cache.clone();
        let second_flush_result = second_flush_result.clone();
        let second_flush_finished = second_flush_finished.clone();
        ThreadOptions::new(move || {
            *second_flush_result.lock() = Some(page_cache.read().flush_range(0..PAGE_SIZE));
            *second_flush_finished.lock() = true;
        })
        .spawn()
    };

    // The second flush prepares the re-dirtied page as clean and holds its
    // lock while waiting for the first writeback. Neither can finish until
    // this test completes the deferred I/O below.
    base::wait_until(
        || {
            second_flush_result.lock().is_some()
                || (page.state().is_up_to_date() && page.try_lock_guard().is_none())
        },
        "timed out waiting for the second flush to wait on page 0 writeback",
    );
    assert!(second_flush_result.lock().is_none());
    assert_eq!(backend.write_count(0), 1);
    assert!(!*second_flush_finished.lock());

    // Complete the first and second writebacks one by one. This exercises the
    // ownership hand-off in async writeback and the wait-for-writeback path.
    assert!(backend.complete_next_deferred_bio(IoKind::Write, true));
    backend.wait_for_deferred_bios(IoKind::Write, 1);
    assert_eq!(backend.write_count(0), 2);
    assert!(!*second_flush_finished.lock());

    assert!(backend.complete_next_deferred_bio(IoKind::Write, true));
    first_flush_thread.join();
    second_flush_thread.join();

    assert!(first_flush_result.lock().take().unwrap().is_ok());
    assert!(second_flush_result.lock().take().unwrap().is_ok());
    assert_eq!(backend.persisted_page_bytes(0), latest_dirty_pattern);
}
