// SPDX-License-Identifier: MPL-2.0

//! Buffered-I/O ordering and shared cold reads.
//!
//! Callers provide an inode-style lock to order conflicting buffered accesses.

use alloc::vec;

use ostd::{mm::VmIo, prelude::ktest};

use super::base::{self, IoCompletion, IoKind, MockPageCacheBackend};
use crate::{prelude::*, thread::kernel_thread::ThreadOptions};

/// Serializes a cold read and a later overwrite with the caller-provided
/// buffered-I/O lock required by the page-cache synchronization model.
#[ktest]
fn read_before_overwrite() {
    let backend = MockPageCacheBackend::new(1);
    backend.set_completion(IoKind::Read, IoCompletion::Deferred);

    let old_pattern = vec![0x3c; PAGE_SIZE];
    let new_pattern = vec![0xa5; PAGE_SIZE];
    backend.set_persisted_page_bytes(0, &old_pattern);

    let page_cache = Arc::new(Mutex::new(base::new_backend_page_cache(&backend, 1)));
    let observed_read_result = Arc::new(Mutex::new(None::<Vec<u8>>));
    let writer_started = Arc::new(Mutex::new(false));
    let writer_finished = Arc::new(Mutex::new(false));

    // Start a cold-page read and hold backend completion so the read keeps the
    // caller-provided buffered-I/O lock while the writer attempts to enter.
    let read_thread = {
        let page_cache = page_cache.clone();
        let observed_read_result = observed_read_result.clone();
        ThreadOptions::new(move || {
            let page_cache = page_cache.lock();
            let mut read_buffer = vec![0; PAGE_SIZE];
            page_cache.read_bytes(0, &mut read_buffer).unwrap();
            *observed_read_result.lock() = Some(read_buffer);
        })
        .spawn()
    };

    backend.wait_for_deferred_bios(IoKind::Read, 1);
    assert_eq!(backend.read_count(0), 1);

    // Issue a full-page overwrite against the same range. The page-cache layer
    // relies on this higher-level lock for buffered read/write ordering, so the
    // writer must not enter `write_bytes()` until the read-side critical section
    // has completed.
    let write_thread = {
        let page_cache = page_cache.clone();
        let writer_started = writer_started.clone();
        let writer_finished = writer_finished.clone();
        let new_pattern = new_pattern.clone();
        ThreadOptions::new(move || {
            *writer_started.lock() = true;
            let page_cache = page_cache.lock();
            page_cache.write_bytes(0, &new_pattern).unwrap();
            *writer_finished.lock() = true;
        })
        .spawn()
    };

    base::wait_until(
        || *writer_started.lock(),
        "timed out waiting for the buffered writer to start",
    );
    assert!(!*writer_finished.lock());

    // Finish the backend read. The serialized reader observes the persisted old
    // bytes, and the later writer's overwrite becomes visible afterwards.
    assert!(backend.complete_next_deferred_bio(IoKind::Read, true));
    read_thread.join();
    write_thread.join();

    assert_eq!(&*observed_read_result.lock(), &Some(old_pattern));
    assert!(*writer_finished.lock());
    assert_eq!(backend.read_count(0), 1);

    let mut read_buffer = vec![0; PAGE_SIZE];
    page_cache.lock().read_bytes(0, &mut read_buffer).unwrap();
    assert_eq!(read_buffer, new_pattern);
}

/// Shares one backend read between two buffered readers of a cold page.
#[ktest]
fn shares_cold_read() {
    let backend = MockPageCacheBackend::new(1);
    backend.set_completion(IoKind::Read, IoCompletion::Deferred);
    let persisted_pattern = vec![0x5a; PAGE_SIZE];
    backend.set_persisted_page_bytes(0, &persisted_pattern);
    let page_cache = base::new_locked_backend_page_cache(&backend, 1);
    let first_read_result = Arc::new(Mutex::new(None::<Vec<u8>>));
    let second_read_result = Arc::new(Mutex::new(None::<Vec<u8>>));

    // Start two cold-page readers while read completion is delayed. They
    // should share one backend read and both see the initialized page.
    let first_reader = {
        let page_cache = page_cache.clone();
        let first_read_result = first_read_result.clone();
        ThreadOptions::new(move || {
            let mut read_buffer = vec![0; PAGE_SIZE];
            page_cache.read().read_bytes(0, &mut read_buffer).unwrap();
            *first_read_result.lock() = Some(read_buffer);
        })
        .spawn()
    };
    let second_reader = {
        let page_cache = page_cache.clone();
        let second_read_result = second_read_result.clone();
        ThreadOptions::new(move || {
            let mut read_buffer = vec![0; PAGE_SIZE];
            page_cache.read().read_bytes(0, &mut read_buffer).unwrap();
            *second_read_result.lock() = Some(read_buffer);
        })
        .spawn()
    };

    backend.wait_for_deferred_bios(IoKind::Read, 1);
    assert_eq!(backend.read_count(0), 1);
    assert!(backend.complete_next_deferred_bio(IoKind::Read, true));

    first_reader.join();
    second_reader.join();

    assert_eq!(&*first_read_result.lock(), &Some(persisted_pattern.clone()));
    assert_eq!(&*second_read_result.lock(), &Some(persisted_pattern));
}
