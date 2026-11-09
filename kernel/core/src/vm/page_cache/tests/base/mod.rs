// SPDX-License-Identifier: MPL-2.0

//! Shared cache construction, mapped-write simulation, and bounded waits.

use ostd::mm::io::util::HasVmReaderWriter;

use crate::{
    prelude::*,
    thread::Thread,
    vm::page_cache::{PageCache, PageCacheBackend, Vmo, VmoMapMode},
};

mod backend;

pub(in crate::vm::page_cache::tests) use backend::{IoCompletion, IoKind, MockPageCacheBackend};

/// Maximum number of cooperative yields before a test wait times out.
const MAX_WAIT_YIELDS: usize = 1_000_000;

/// Creates a page cache with `num_pages` backend pages for test scenarios.
pub(in crate::vm::page_cache::tests) fn new_backend_page_cache(
    backend: &Arc<MockPageCacheBackend>,
    num_pages: usize,
) -> PageCache {
    let backend_dyn: Arc<dyn PageCacheBackend> = backend.clone();
    PageCache::new_with_backend(num_pages * PAGE_SIZE, Arc::downgrade(&backend_dyn)).unwrap()
}

/// Creates a locked page cache that models the inode-level lock held by
/// filesystem callers before touching the page cache.
pub(in crate::vm::page_cache::tests) fn new_locked_backend_page_cache(
    backend: &Arc<MockPageCacheBackend>,
    num_pages: usize,
) -> Arc<RwMutex<PageCache>> {
    Arc::new(RwMutex::new(new_backend_page_cache(backend, num_pages)))
}

/// Simulates shared mapped stores to the first VMO page without a page table.
///
/// A store page fault first commits page 0 for shared writes, which marks an
/// up-to-date backend page dirty. The actual byte copy then writes the
/// committed frame directly, matching mapped-memory stores without routing
/// through the page-cache write API.
pub(in crate::vm::page_cache::tests) fn write_first_page_via_vmo(vmo: &Vmo, bytes: &[u8]) {
    assert!(!bytes.is_empty());
    assert!(bytes.len() <= PAGE_SIZE);

    vmo.commit_on(0, VmoMapMode::SharedWrite).unwrap();
    let (page, _) = vmo.try_commit_page(0, VmoMapMode::SharedWrite).unwrap();

    let mut reader = VmReader::from(bytes);
    let mut writer = page.writer();
    assert_eq!(writer.write(&mut reader), bytes.len());
}

/// Waits until a test predicate becomes true.
#[track_caller]
pub(in crate::vm::page_cache::tests) fn wait_until(
    mut condition: impl FnMut() -> bool,
    failure_message: &str,
) {
    for _ in 0..MAX_WAIT_YIELDS {
        if condition() {
            return;
        }

        Thread::yield_now();
    }

    panic!("{failure_message}");
}
