// SPDX-License-Identifier: MPL-2.0

//! Tests for clean-page eviction and dirty-page retention during invalidation.
//!
//! Flushing supplies a clean page or an in-flight writeback as preparation;
//! these cases assert whether eviction removes or retains the cached data.

use alloc::vec;

use ostd::{mm::VmIo, prelude::ktest};

use super::base::{self, IoCompletion, IoKind, MockPageCacheBackend};
use crate::{
    prelude::*,
    thread::kernel_thread::ThreadOptions,
    vm::page_cache::{CachePageExt, VmoMapMode},
};

/// Leaves successfully written pages clean, unlocked, and eligible for eviction.
#[ktest]
fn reloads_clean_page() {
    let backend = MockPageCacheBackend::new(1);
    let page_cache = base::new_backend_page_cache(&backend, 1);
    let pattern = vec![0x45; PAGE_SIZE];
    page_cache.write_bytes(0, &pattern).unwrap();

    page_cache.flush_range(0..PAGE_SIZE).unwrap();

    assert_eq!(backend.write_count(0), 1);
    assert_eq!(backend.persisted_page_bytes(0), pattern);
    let (page, _) = page_cache
        .as_vmo()
        .try_commit_page(0, VmoMapMode::SharedRead)
        .unwrap();
    {
        let locked_page = page
            .try_lock_guard()
            .expect("completed page is still locked");
        assert!(locked_page.state().is_up_to_date());
        assert!(!locked_page.is_writing_back());
    }

    page_cache.evict_range(0..PAGE_SIZE).unwrap();
    let refreshed_pattern = vec![0x67; PAGE_SIZE];
    backend.set_persisted_page_bytes(0, &refreshed_pattern);
    let mut read_buffer = vec![0; PAGE_SIZE];
    page_cache.read_bytes(0, &mut read_buffer).unwrap();
    assert_eq!(backend.read_count(0), 1);
    assert_eq!(read_buffer, refreshed_pattern);
}

/// Retains a page re-dirtied before invalidation reaches its eviction step.
#[ktest]
fn invalidation_keeps_redirty() {
    let backend = MockPageCacheBackend::new(1);
    backend.set_completion(IoKind::Write, IoCompletion::Deferred);
    let page_cache = base::new_locked_backend_page_cache(&backend, 1);
    let vmo = page_cache.read().as_vmo().clone();
    let first_pattern = vec![0x11; PAGE_SIZE];
    let latest_pattern = vec![0x33; PAGE_SIZE];
    page_cache.write().write_bytes(0, &first_pattern).unwrap();
    let (page, _) = vmo.try_commit_page(0, VmoMapMode::SharedRead).unwrap();

    let result = Arc::new(Mutex::new(None::<Result<()>>));
    let invalidate_thread = {
        let page_cache = page_cache.clone();
        let result = result.clone();
        ThreadOptions::new(move || {
            *result.lock() = Some(page_cache.read().invalidate_range(0..PAGE_SIZE));
        })
        .spawn()
    };

    backend.wait_for_deferred_bios(IoKind::Write, 1);
    // A mapped store bypasses the outer lock while invalidation waits for I/O.
    base::write_first_page_via_vmo(&vmo, &latest_pattern);
    assert!(page.state().is_dirty());
    assert!(backend.complete_next_deferred_bio(IoKind::Write, true));
    invalidate_thread.join();

    assert!(result.lock().take().unwrap().is_ok());
    {
        let locked_page = page.lock_guard();
        assert!(locked_page.state().is_dirty());
        assert!(!locked_page.is_writing_back());
    }
    let mut read_buffer = vec![0; PAGE_SIZE];
    page_cache.read().read_bytes(0, &mut read_buffer).unwrap();
    assert_eq!(read_buffer, latest_pattern);
    assert_eq!(backend.read_count(0), 0);
    assert_eq!(backend.write_count(0), 1);

    // The page retained by invalidation must remain eligible for writeback.
    backend.set_completion(IoKind::Write, IoCompletion::Immediate);
    page_cache.read().flush_range(0..PAGE_SIZE).unwrap();
    assert_eq!(backend.write_count(0), 2);
    assert_eq!(backend.persisted_page_bytes(0), latest_pattern);
}
