// SPDX-License-Identifier: MPL-2.0

//! Resizing coordinates with in-flight VMO commits.
//!
//! Resize holds the caller's exclusive inode-style lock; VMO commits bypass it.

use alloc::vec;

use ostd::{mm::VmIo, prelude::ktest};

use super::base::{self, IoCompletion, IoKind, MockPageCacheBackend};
use crate::{prelude::*, thread::kernel_thread::ThreadOptions, vm::page_cache::VmoMapMode};

/// Commits a page while a concurrent truncate shrinks the page cache, ensuring
/// pages beyond the new size stay inaccessible.
#[ktest]
fn truncate_after_commit() {
    let backend = MockPageCacheBackend::new(2);
    backend.set_completion(IoKind::Read, IoCompletion::Deferred);
    backend.set_persisted_page_bytes(1, &[0x9b; PAGE_SIZE]);

    let page_cache = base::new_locked_backend_page_cache(&backend, 2);
    let vmo = page_cache.read().as_vmo().clone();
    let commit_second_page_result = Arc::new(Mutex::new(None::<Result<()>>));
    let resize_result = Arc::new(Mutex::new(None::<Result<()>>));
    let resize_finished = Arc::new(Mutex::new(false));

    // Commit page 1 and pause its backend read so truncate can shrink the VMO
    // while that commit is still waiting for initialization to finish.
    let commit_thread = {
        let vmo = vmo.clone();
        let commit_second_page_result = commit_second_page_result.clone();
        ThreadOptions::new(move || {
            *commit_second_page_result.lock() =
                Some(vmo.commit_on(1, VmoMapMode::SharedRead).map(|_| ()));
        })
        .spawn()
    };

    backend.wait_for_deferred_bios(IoKind::Read, 1);

    let resize_thread = {
        let page_cache = page_cache.clone();
        let resize_result = resize_result.clone();
        let resize_finished = resize_finished.clone();
        ThreadOptions::new(move || {
            *resize_result.lock() = Some(page_cache.write().resize(PAGE_SIZE, 2 * PAGE_SIZE));
            *resize_finished.lock() = true;
        })
        .spawn()
    };

    // Shrinking publishes the new capacity before waiting for the removed
    // page's initialization. Observe that step rather than thread startup.
    base::wait_until(
        || vmo.size() == PAGE_SIZE || *resize_finished.lock(),
        "timed out waiting for truncate to publish the one-page capacity",
    );
    assert_eq!(vmo.size(), PAGE_SIZE);
    assert!(!*resize_finished.lock());

    // Let the blocked commit finish, then verify the truncated page is no
    // longer accessible through subsequent VMO operations.
    assert!(backend.complete_next_deferred_bio(IoKind::Read, true));
    commit_thread.join();
    resize_thread.join();

    assert!(commit_second_page_result.lock().take().unwrap().is_ok());
    assert!(resize_result.lock().take().unwrap().is_ok());
    assert_eq!(
        vmo.commit_on(1, VmoMapMode::SharedRead)
            .unwrap_err()
            .error(),
        Errno::EINVAL
    );

    let mut read_buffer = vec![0; PAGE_SIZE];
    page_cache
        .read()
        .read_bytes(PAGE_SIZE, &mut read_buffer)
        .unwrap();
    assert_eq!(read_buffer, vec![0; PAGE_SIZE]);
}
