// SPDX-License-Identifier: MPL-2.0

//! Regression tests for the the concurrency scenarios for the
//! page-cache subsystem.

use alloc::{sync::Arc, vec, vec::Vec};

use ostd::{mm::VmIo, prelude::ktest, sync::Mutex};

use self::utils::{IoCompletion, IoKind, MockPageCacheBackend, wait_until};
use super::{PageCache, PageCacheBackend, VmoCommitError};
use crate::{prelude::*, thread::kernel_thread::ThreadOptions};

mod utils;

/// Creates a disk-backed page cache with `num_pages` pages for test scenarios.
fn new_disk_backed_page_cache(backend: &Arc<MockPageCacheBackend>, num_pages: usize) -> PageCache {
    let backend_dyn: Arc<dyn PageCacheBackend> = backend.clone();
    PageCache::new_disk_backed(num_pages * PAGE_SIZE, Arc::downgrade(&backend_dyn)).unwrap()
}

/// Reads a cold page while a concurrent overwrite waits, so readers only see
/// complete page versions and the later write becomes visible.
#[ktest]
fn concurrent_read_and_write() {
    let backend = MockPageCacheBackend::new(1);
    backend.set_completion(IoKind::Read, IoCompletion::Deferred);

    let old_pattern = vec![0x3c; PAGE_SIZE];
    let new_pattern = vec![0xa5; PAGE_SIZE];
    backend.set_persisted_page_bytes(0, &old_pattern);

    let page_cache = new_disk_backed_page_cache(&backend, 1);
    let vmo = page_cache.as_vmo();
    let observed_read_result = Arc::new(Mutex::new(None::<Vec<u8>>));
    let writer_started = Arc::new(Mutex::new(false));
    let writer_finished = Arc::new(Mutex::new(false));

    // Start a cold-page read and hold backend completion so the page stays in
    // the initialization path while the writer races with it.
    let read_thread = {
        let vmo = vmo.clone();
        let observed_read_result = observed_read_result.clone();
        ThreadOptions::new(move || {
            let mut read_buffer = vec![0; PAGE_SIZE];
            vmo.read_bytes(0, &mut read_buffer).unwrap();
            *observed_read_result.lock() = Some(read_buffer);
        })
        .spawn()
    };

    backend.wait_for_deferred_bios(IoKind::Read, 1);
    assert_eq!(backend.read_count(0), 1);

    // Issue a full-page overwrite against the same range. This writer should
    // wait for initialization instead of exposing a mixed old/new page image.
    let write_thread = {
        let vmo = vmo.clone();
        let writer_started = writer_started.clone();
        let writer_finished = writer_finished.clone();
        let new_pattern = new_pattern.clone();
        ThreadOptions::new(move || {
            *writer_started.lock() = true;
            vmo.write_bytes(0, &new_pattern).unwrap();
            *writer_finished.lock() = true;
        })
        .spawn()
    };

    wait_until(|| *writer_started.lock());
    assert!(!*writer_finished.lock());

    // Finish the backend read and then verify the reader saw the old version
    // while later reads observe the completed overwrite.
    assert!(backend.complete_next_deferred_bio(IoKind::Read, true));
    read_thread.join();
    write_thread.join();

    assert_eq!(&*observed_read_result.lock(), &Some(old_pattern));
    assert!(*writer_finished.lock());
    assert_eq!(backend.read_count(0), 1);

    let mut read_buffer = vec![0; PAGE_SIZE];
    vmo.read_bytes(0, &mut read_buffer).unwrap();
    assert_eq!(read_buffer, new_pattern);
}

/// Flushes a dirty page while another task re-dirties it, so writeback reaches
/// the backend and the newest dirty bytes are not silently lost.
#[ktest]
fn concurrent_write_and_flush() {
    let backend = MockPageCacheBackend::new(1);
    backend.set_completion(IoKind::Write, IoCompletion::Deferred);

    let page_cache = new_disk_backed_page_cache(&backend, 1);
    let first_dirty_pattern = vec![0x11; PAGE_SIZE];
    let latest_dirty_pattern = vec![0x22; PAGE_SIZE];
    page_cache.write_bytes(0, &first_dirty_pattern).unwrap();

    let flush_result = Arc::new(Mutex::new(None::<Result<()>>));
    let writer_finished = Arc::new(Mutex::new(false));

    // Start writeback and pin it in the deferred state so a concurrent writer
    // can dirty the page again before the first flush completes.
    let flush_thread = {
        let page_cache = page_cache.clone();
        let flush_result = flush_result.clone();
        ThreadOptions::new(move || {
            *flush_result.lock() = Some(page_cache.flush_range(0..PAGE_SIZE));
        })
        .spawn()
    };

    backend.wait_for_deferred_bios(IoKind::Write, 1);

    // Re-dirty the same page while the first writeback is in flight. A later
    // flush must persist this newest version instead of silently dropping it.
    let writer_thread = {
        let page_cache = page_cache.clone();
        let writer_finished = writer_finished.clone();
        let latest_dirty_pattern = latest_dirty_pattern.clone();
        ThreadOptions::new(move || {
            page_cache.write_bytes(0, &latest_dirty_pattern).unwrap();
            *writer_finished.lock() = true;
        })
        .spawn()
    };

    writer_thread.join();
    assert!(*writer_finished.lock());

    // Complete the first writeback, then flush again and confirm the backend
    // eventually stores the latest dirty bytes.
    assert!(backend.complete_next_deferred_bio(IoKind::Write, true));
    flush_thread.join();
    assert!(flush_result.lock().take().unwrap().is_ok());

    backend.set_completion(IoKind::Write, IoCompletion::Immediate);
    page_cache.flush_range(0..PAGE_SIZE).unwrap();

    let mut read_buffer = vec![0; PAGE_SIZE];
    page_cache.read_bytes(0, &mut read_buffer).unwrap();
    assert_eq!(read_buffer, latest_dirty_pattern);
    assert_eq!(backend.write_count(0), 2);
    assert_eq!(backend.persisted_page_bytes(0), latest_dirty_pattern);
}

/// Re-dirties a page while another task runs `flush_range()` and
/// `evict_range()`, ensuring the newest dirty page is kept cached.
#[ktest]
fn concurrent_write_and_evict() {
    let backend = MockPageCacheBackend::new(1);
    backend.set_completion(IoKind::Write, IoCompletion::Deferred);

    let page_cache = new_disk_backed_page_cache(&backend, 1);
    let first_dirty_pattern = vec![0x52; PAGE_SIZE];
    let latest_dirty_pattern = vec![0x7d; PAGE_SIZE];
    page_cache.write_bytes(0, &first_dirty_pattern).unwrap();

    // Race a flush+evict sequence against a new writer after writeback has
    // already started. This checks that a page re-dirtied before eviction
    // stays cached instead of being silently dropped.
    let flush_and_evict_result = Arc::new(Mutex::new(None::<Result<()>>));
    let flush_and_evict_thread = {
        let page_cache = page_cache.clone();
        let flush_and_evict_result = flush_and_evict_result.clone();
        ThreadOptions::new(move || {
            let result = page_cache
                .flush_range(0..PAGE_SIZE)
                .and_then(|()| page_cache.evict_range(0..PAGE_SIZE));
            *flush_and_evict_result.lock() = Some(result);
        })
        .spawn()
    };

    backend.wait_for_deferred_bios(IoKind::Write, 1);

    // Dirty the page again before the first writeback completes. Eviction
    // should leave this newest dirty page resident in cache.
    let writer_thread = {
        let page_cache = page_cache.clone();
        let latest_dirty_pattern = latest_dirty_pattern.clone();
        ThreadOptions::new(move || {
            page_cache.write_bytes(0, &latest_dirty_pattern).unwrap();
        })
        .spawn()
    };

    writer_thread.join();

    // After the first writeback finishes, the flush+evict path should return,
    // but the re-dirtied page must still be readable from cache.
    assert!(backend.complete_next_deferred_bio(IoKind::Write, true));
    flush_and_evict_thread.join();
    assert!(flush_and_evict_result.lock().take().unwrap().is_ok());

    let mut read_buffer = vec![0; PAGE_SIZE];
    page_cache.read_bytes(0, &mut read_buffer).unwrap();
    assert_eq!(read_buffer, latest_dirty_pattern);
    assert_eq!(backend.read_count(0), 0);

    backend.set_completion(IoKind::Write, IoCompletion::Immediate);
    page_cache.flush_range(0..PAGE_SIZE).unwrap();
    assert_eq!(backend.write_count(0), 2);
    assert_eq!(backend.persisted_page_bytes(0), latest_dirty_pattern);
}

/// Commits a page while a concurrent truncate shrinks the page cache, ensuring
/// pages beyond the new size stay inaccessible.
#[ktest]
fn concurrent_commit_and_truncate() {
    let backend = MockPageCacheBackend::new(2);
    backend.set_completion(IoKind::Read, IoCompletion::Deferred);
    backend.set_persisted_page_bytes(1, &[0x9b; PAGE_SIZE]);

    let page_cache = new_disk_backed_page_cache(&backend, 2);
    let commit_second_page_result = Arc::new(Mutex::new(None::<Result<()>>));

    // Commit page 1 and pause its backend read so truncate can shrink the VMO
    // while that commit is still waiting for initialization to finish.
    let commit_thread = {
        let vmo = page_cache.as_vmo();
        let commit_second_page_result = commit_second_page_result.clone();
        ThreadOptions::new(move || {
            *commit_second_page_result.lock() = Some(vmo.commit_on(1).map(|_| ()));
        })
        .spawn()
    };

    backend.wait_for_deferred_bios(IoKind::Read, 1);
    page_cache.resize(PAGE_SIZE, 2 * PAGE_SIZE).unwrap();

    // Let the blocked commit finish, then verify the truncated page is no
    // longer accessible through subsequent VMO operations.
    assert!(backend.complete_next_deferred_bio(IoKind::Read, true));
    commit_thread.join();

    assert!(commit_second_page_result.lock().take().unwrap().is_ok());
    assert_eq!(
        page_cache.as_vmo().commit_on(1).unwrap_err().error(),
        Errno::EINVAL
    );

    let mut read_buffer = vec![0; PAGE_SIZE];
    page_cache.read_bytes(PAGE_SIZE, &mut read_buffer).unwrap();
    assert_eq!(read_buffer, vec![0; PAGE_SIZE]);
}

/// Faults the same cold page through `try_commit_page()` and `commit_on()`,
/// verifying that one backend read initializes the page for all waiters.
#[ktest]
fn concurrent_page_faults() {
    let backend = MockPageCacheBackend::new(1);
    backend.set_completion(IoKind::Read, IoCompletion::Deferred);
    backend.set_persisted_page_bytes(0, &[0x6b; PAGE_SIZE]);

    let page_cache = new_disk_backed_page_cache(&backend, 1);
    let vmo = page_cache.as_vmo();
    // The first page-fault style probe should report that backend I/O is
    // needed because the page has not been committed yet.
    assert!(matches!(
        vmo.try_commit_page(0),
        Err(VmoCommitError::NeedIo(0))
    ));

    let first_commit_finished = Arc::new(Mutex::new(false));
    let second_commit_finished = Arc::new(Mutex::new(false));

    // Start one blocking commit to issue the backend read. Other faulting
    // callers for the same page should observe the in-progress initialization.
    let first_thread = {
        let vmo = vmo.clone();
        let first_commit_finished = first_commit_finished.clone();
        ThreadOptions::new(move || {
            vmo.commit_on(0).unwrap();
            *first_commit_finished.lock() = true;
        })
        .spawn()
    };

    backend.wait_for_deferred_bios(IoKind::Read, 1);
    match vmo.try_commit_page(0) {
        Err(VmoCommitError::WaitUntilInit(0, _)) => {}
        other => panic!("unexpected page-fault state: {other:?}"),
    }

    // A second blocking commit should join the same initialization instead of
    // submitting a duplicate read BIO for the same page.
    let second_thread = {
        let vmo = vmo.clone();
        let second_commit_finished = second_commit_finished.clone();
        ThreadOptions::new(move || {
            vmo.commit_on(0).unwrap();
            *second_commit_finished.lock() = true;
        })
        .spawn()
    };

    assert_eq!(backend.read_count(0), 1);
    assert!(!*first_commit_finished.lock());
    assert!(!*second_commit_finished.lock());

    // Release the single deferred read and check that both waiters complete
    // and observe the initialized page contents.
    assert!(backend.complete_next_deferred_bio(IoKind::Read, true));
    first_thread.join();
    second_thread.join();

    assert!(*first_commit_finished.lock());
    assert!(*second_commit_finished.lock());
    assert_eq!(backend.read_count(0), 1);

    let mut read_buffer = vec![0; PAGE_SIZE];
    vmo.read_bytes(0, &mut read_buffer).unwrap();
    assert_eq!(read_buffer, vec![0x6b; PAGE_SIZE]);
}

/// Keeps backend reads failing for one page and checks both the initial
/// committer and a later waiter return `EIO` instead of livelocking.
#[ktest]
fn persistent_backend_errors() {
    let backend = MockPageCacheBackend::new(1);
    backend.set_completion(IoKind::Read, IoCompletion::Deferred);

    let page_cache = new_disk_backed_page_cache(&backend, 1);
    let vmo = page_cache.as_vmo();
    let first_error = Arc::new(Mutex::new(None::<Errno>));
    let second_error = Arc::new(Mutex::new(None::<Errno>));

    // Start one commit that blocks on the backend read. Once that read fails
    // and the first caller has fully returned, issue a second commit for the
    // same page to retry the initialization path.
    let first_thread = {
        let vmo = vmo.clone();
        let first_error = first_error.clone();
        ThreadOptions::new(move || {
            *first_error.lock() = Some(vmo.commit_on(0).unwrap_err().error());
        })
        .spawn()
    };

    backend.wait_for_deferred_bios(IoKind::Read, 1);

    // Fail the first backend read and wait until the first caller has fully
    // unwound, including releasing the DMA mapping used by the failed BIO.
    assert!(backend.complete_next_deferred_bio(IoKind::Read, false));
    first_thread.join();

    let second_thread = {
        let vmo = vmo.clone();
        let second_error = second_error.clone();
        ThreadOptions::new(move || {
            *second_error.lock() = Some(vmo.commit_on(0).unwrap_err().error());
        })
        .spawn()
    };

    // Fail the retry as well. This keeps the regression focused on repeated
    // backend errors while avoiding overlap between two failed DMA mappings.
    backend.wait_for_deferred_bios(IoKind::Read, 1);
    assert!(backend.complete_next_deferred_bio(IoKind::Read, false));
    second_thread.join();

    assert_eq!(&*first_error.lock(), &Some(Errno::EIO));
    assert_eq!(&*second_error.lock(), &Some(Errno::EIO));
    assert_eq!(backend.read_count(0), 2);
}

/// Delays BIO completion on demand to exercise shared read I/O, writeback
/// hand-off, and waiting for `is_writing_back` to clear.
#[ktest]
fn delayed_io_completion() {
    let backend = MockPageCacheBackend::new(1);
    backend.set_completion(IoKind::Read, IoCompletion::Deferred);
    backend.set_completion(IoKind::Write, IoCompletion::Deferred);

    let persisted_pattern = vec![0x5a; PAGE_SIZE];
    let first_dirty_pattern = vec![0x11; PAGE_SIZE];
    let latest_dirty_pattern = vec![0x33; PAGE_SIZE];
    backend.set_persisted_page_bytes(0, &persisted_pattern);

    let page_cache = new_disk_backed_page_cache(&backend, 1);
    let first_read_result = Arc::new(Mutex::new(None::<Vec<u8>>));
    let second_read_result = Arc::new(Mutex::new(None::<Vec<u8>>));

    // Start two cold-page readers while read completion is delayed. They
    // should share one backend read and both see the initialized page.
    let first_reader = {
        let page_cache = page_cache.clone();
        let first_read_result = first_read_result.clone();
        ThreadOptions::new(move || {
            let mut read_buffer = vec![0; PAGE_SIZE];
            page_cache.read_bytes(0, &mut read_buffer).unwrap();
            *first_read_result.lock() = Some(read_buffer);
        })
        .spawn()
    };
    let second_reader = {
        let page_cache = page_cache.clone();
        let second_read_result = second_read_result.clone();
        ThreadOptions::new(move || {
            let mut read_buffer = vec![0; PAGE_SIZE];
            page_cache.read_bytes(0, &mut read_buffer).unwrap();
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

    // Start one deferred writeback, then dirty the page again before it
    // completes so a second flush must wait for `is_writing_back` to clear.
    page_cache.write_bytes(0, &first_dirty_pattern).unwrap();

    let first_flush_result = Arc::new(Mutex::new(None::<Result<()>>));
    let second_flush_result = Arc::new(Mutex::new(None::<Result<()>>));
    let second_flush_started = Arc::new(Mutex::new(false));
    let second_flush_finished = Arc::new(Mutex::new(false));

    let first_flush_thread = {
        let page_cache = page_cache.clone();
        let first_flush_result = first_flush_result.clone();
        ThreadOptions::new(move || {
            *first_flush_result.lock() = Some(page_cache.flush_range(0..PAGE_SIZE));
        })
        .spawn()
    };

    backend.wait_for_deferred_bios(IoKind::Write, 1);

    page_cache.write_bytes(0, &latest_dirty_pattern).unwrap();

    let second_flush_thread = {
        let page_cache = page_cache.clone();
        let second_flush_result = second_flush_result.clone();
        let second_flush_started = second_flush_started.clone();
        let second_flush_finished = second_flush_finished.clone();
        ThreadOptions::new(move || {
            *second_flush_started.lock() = true;
            *second_flush_result.lock() = Some(page_cache.flush_range(0..PAGE_SIZE));
            *second_flush_finished.lock() = true;
        })
        .spawn()
    };

    wait_until(|| *second_flush_started.lock());
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
