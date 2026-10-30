// SPDX-License-Identifier: MPL-2.0

//! Regression tests for page-cache concurrency and readahead.

use alloc::vec;

use ostd::{
    mm::{VmIo, io::util::HasVmReaderWriter},
    prelude::ktest,
};

use self::utils::{IoCompletion, IoKind, MockPageCacheBackend, try_wait_until, wait_until};
use super::{
    PageCache, PageCacheBackend, Vmo, VmoCommitError, VmoMapMode,
    readahead::{Readahead, ReadaheadTrigger},
};
use crate::{prelude::*, thread::kernel_thread::ThreadOptions};

mod utils;

/// Creates a page cache with `num_pages` backend pages for test scenarios.
fn new_backend_page_cache(backend: &Arc<MockPageCacheBackend>, num_pages: usize) -> PageCache {
    let backend_dyn: Arc<dyn PageCacheBackend> = backend.clone();
    PageCache::new_with_backend(num_pages * PAGE_SIZE, Arc::downgrade(&backend_dyn)).unwrap()
}

/// Creates a locked page cache that models the inode-level lock held by
/// filesystem callers before touching the page cache.
fn new_locked_backend_page_cache(
    backend: &Arc<MockPageCacheBackend>,
    num_pages: usize,
) -> Arc<RwMutex<PageCache>> {
    Arc::new(RwMutex::new(new_backend_page_cache(backend, num_pages)))
}

/// Simulates stores through a shared writable VMO mapping.
///
/// A store page fault first commits page 0 for shared writes, which marks an
/// up-to-date backend page dirty. The actual byte copy then writes the
/// committed frame directly, matching mapped-memory stores without routing
/// through the page-cache write API.
fn write_vmo_bytes(vmo: &Vmo, bytes: &[u8]) {
    assert!(!bytes.is_empty());
    assert!(bytes.len() <= PAGE_SIZE);

    vmo.commit_on(0, VmoMapMode::SharedWrite).unwrap();
    let (page, _) = vmo.try_commit_page(0, VmoMapMode::SharedWrite).unwrap();

    let mut reader = VmReader::from(bytes);
    let mut writer = page.writer();
    assert_eq!(writer.write(&mut reader), bytes.len());
}

/// Serializes a cold read and a later overwrite with the caller-provided
/// buffered-I/O lock required by the page-cache synchronization model.
#[ktest]
fn concurrent_read_and_write() {
    let backend = MockPageCacheBackend::new(1);
    backend.set_completion(IoKind::Read, IoCompletion::Deferred);

    let old_pattern = vec![0x3c; PAGE_SIZE];
    let new_pattern = vec![0xa5; PAGE_SIZE];
    backend.set_persisted_page_bytes(0, &old_pattern);

    let page_cache = Arc::new(Mutex::new(new_backend_page_cache(&backend, 1)));
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

    wait_until(|| *writer_started.lock());
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

/// Flushes a dirty page while another task re-dirties it, so writeback reaches
/// the backend and the newest dirty bytes are not silently lost.
#[ktest]
fn concurrent_write_and_flush() {
    let backend = MockPageCacheBackend::new(1);
    backend.set_completion(IoKind::Write, IoCompletion::Deferred);

    let page_cache = new_locked_backend_page_cache(&backend, 1);
    let vmo = page_cache.read().as_vmo().clone();
    let first_dirty_pattern = vec![0x11; PAGE_SIZE];
    let latest_dirty_pattern = vec![0x22; PAGE_SIZE];
    page_cache
        .write()
        .write_bytes(0, &first_dirty_pattern)
        .unwrap();

    let flush_result = Arc::new(Mutex::new(None::<Result<()>>));
    let writer_finished = Arc::new(Mutex::new(false));

    // Start writeback and pin it in the deferred state so a concurrent writer
    // can dirty the page again before the first flush completes.
    let flush_thread = {
        let page_cache = page_cache.clone();
        let flush_result = flush_result.clone();
        ThreadOptions::new(move || {
            *flush_result.lock() = Some(page_cache.read().flush_range(0..PAGE_SIZE));
        })
        .spawn()
    };

    backend.wait_for_deferred_bios(IoKind::Write, 1);

    // Re-dirty the same page while the first writeback is in flight. A later
    // flush must persist this newest version instead of silently dropping it.
    let writer_thread = {
        let vmo = vmo.clone();
        let writer_finished = writer_finished.clone();
        let latest_dirty_pattern = latest_dirty_pattern.clone();
        ThreadOptions::new(move || {
            write_vmo_bytes(&vmo, &latest_dirty_pattern);
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
    page_cache.read().flush_range(0..PAGE_SIZE).unwrap();

    let mut read_buffer = vec![0; PAGE_SIZE];
    page_cache.read().read_bytes(0, &mut read_buffer).unwrap();
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

    let page_cache = new_locked_backend_page_cache(&backend, 1);
    let vmo = page_cache.read().as_vmo().clone();
    let first_dirty_pattern = vec![0x52; PAGE_SIZE];
    let latest_dirty_pattern = vec![0x7d; PAGE_SIZE];
    page_cache
        .write()
        .write_bytes(0, &first_dirty_pattern)
        .unwrap();

    // Race a flush+evict sequence against a new writer after writeback has
    // already started. This checks that a page re-dirtied before eviction
    // stays cached instead of being silently dropped.
    let flush_and_evict_result = Arc::new(Mutex::new(None::<Result<()>>));
    let flush_and_evict_thread = {
        let page_cache = page_cache.clone();
        let flush_and_evict_result = flush_and_evict_result.clone();
        ThreadOptions::new(move || {
            let page_cache = page_cache.read();
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
        let vmo = vmo.clone();
        let latest_dirty_pattern = latest_dirty_pattern.clone();
        ThreadOptions::new(move || {
            write_vmo_bytes(&vmo, &latest_dirty_pattern);
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
    page_cache.read().read_bytes(0, &mut read_buffer).unwrap();
    assert_eq!(read_buffer, latest_dirty_pattern);
    assert_eq!(backend.read_count(0), 0);

    backend.set_completion(IoKind::Write, IoCompletion::Immediate);
    page_cache.read().flush_range(0..PAGE_SIZE).unwrap();
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

    let page_cache = new_locked_backend_page_cache(&backend, 2);
    let vmo = page_cache.read().as_vmo().clone();
    let commit_second_page_result = Arc::new(Mutex::new(None::<Result<()>>));
    let resize_result = Arc::new(Mutex::new(None::<Result<()>>));
    let resize_started = Arc::new(Mutex::new(false));
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
        let resize_started = resize_started.clone();
        let resize_finished = resize_finished.clone();
        ThreadOptions::new(move || {
            *resize_started.lock() = true;
            *resize_result.lock() = Some(page_cache.write().resize(PAGE_SIZE, 2 * PAGE_SIZE));
            *resize_finished.lock() = true;
        })
        .spawn()
    };

    wait_until(|| *resize_started.lock());
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

/// Faults the same cold page through `try_commit_page()` and `commit_on()`,
/// verifying that one backend read initializes the page for all waiters.
#[ktest]
fn concurrent_page_faults() {
    let backend = MockPageCacheBackend::new(1);
    backend.set_completion(IoKind::Read, IoCompletion::Deferred);
    backend.set_persisted_page_bytes(0, &[0x6b; PAGE_SIZE]);

    let page_cache = new_backend_page_cache(&backend, 1);
    let vmo = page_cache.as_vmo().clone();
    // The first page-fault style probe should report that backend I/O is
    // needed because the page has not been committed yet.
    assert!(matches!(
        vmo.try_commit_page(0, VmoMapMode::SharedRead),
        Err(VmoCommitError::NeedIo { index: 0 })
    ));

    let first_commit_finished = Arc::new(Mutex::new(false));
    let second_commit_finished = Arc::new(Mutex::new(false));

    // Start one blocking commit to issue the backend read. Other faulting
    // callers for the same page should observe the in-progress initialization.
    let first_thread = {
        let vmo = vmo.clone();
        let first_commit_finished = first_commit_finished.clone();
        ThreadOptions::new(move || {
            vmo.commit_on(0, VmoMapMode::SharedRead).unwrap();
            *first_commit_finished.lock() = true;
        })
        .spawn()
    };

    backend.wait_for_deferred_bios(IoKind::Read, 1);
    match vmo.try_commit_page(0, VmoMapMode::SharedRead) {
        Err(VmoCommitError::NeedIo { index: 0 }) => {}
        other => panic!("unexpected page-fault state: {other:?}"),
    }

    // A second blocking commit should join the same initialization instead of
    // submitting a duplicate read BIO for the same page.
    let second_thread = {
        let vmo = vmo.clone();
        let second_commit_finished = second_commit_finished.clone();
        ThreadOptions::new(move || {
            vmo.commit_on(0, VmoMapMode::SharedRead).unwrap();
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
    let mut writer = VmWriter::from(read_buffer.as_mut_slice()).to_fallible();
    vmo.read(0, &mut writer, None).unwrap();
    assert_eq!(read_buffer, vec![0x6b; PAGE_SIZE]);
}

/// Keeps backend reads failing for one page and checks both the initial
/// committer and a later waiter return `EIO` instead of livelocking.
#[ktest]
fn persistent_backend_errors() {
    let backend = MockPageCacheBackend::new(1);
    backend.set_completion(IoKind::Read, IoCompletion::Deferred);

    let page_cache = new_backend_page_cache(&backend, 1);
    let vmo = page_cache.as_vmo().clone();
    let first_error = Arc::new(Mutex::new(None::<Errno>));
    let second_error = Arc::new(Mutex::new(None::<Errno>));

    // Start one commit that blocks on the backend read. A second commit for
    // the same page then goes through `ensure_init()` and should also return
    // promptly once initialization fails.
    let first_thread = {
        let vmo = vmo.clone();
        let first_error = first_error.clone();
        ThreadOptions::new(move || {
            *first_error.lock() = Some(
                vmo.commit_on(0, VmoMapMode::SharedRead)
                    .unwrap_err()
                    .error(),
            );
        })
        .spawn()
    };

    backend.wait_for_deferred_bios(IoKind::Read, 1);

    let second_thread = {
        let vmo = vmo.clone();
        let second_error = second_error.clone();
        ThreadOptions::new(move || {
            *second_error.lock() = Some(
                vmo.commit_on(0, VmoMapMode::SharedRead)
                    .unwrap_err()
                    .error(),
            );
        })
        .spawn()
    };

    // Fail the first backend read, then fail the retry as well. This checks
    // that both callers get `EIO` back rather than spinning forever.
    assert!(backend.complete_next_deferred_bio(IoKind::Read, false));
    backend.wait_for_deferred_bios(IoKind::Read, 1);
    assert!(backend.complete_next_deferred_bio(IoKind::Read, false));

    first_thread.join();
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

    let page_cache = new_locked_backend_page_cache(&backend, 1);
    let vmo = page_cache.read().as_vmo().clone();
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

    // Start one deferred writeback, then dirty the page again before it
    // completes so a second flush must wait for `is_writing_back` to clear.
    page_cache
        .write()
        .write_bytes(0, &first_dirty_pattern)
        .unwrap();

    let first_flush_result = Arc::new(Mutex::new(None::<Result<()>>));
    let second_flush_result = Arc::new(Mutex::new(None::<Result<()>>));
    let second_flush_started = Arc::new(Mutex::new(false));
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

    write_vmo_bytes(&vmo, &latest_dirty_pattern);

    let second_flush_thread = {
        let page_cache = page_cache.clone();
        let second_flush_result = second_flush_result.clone();
        let second_flush_started = second_flush_started.clone();
        let second_flush_finished = second_flush_finished.clone();
        ThreadOptions::new(move || {
            *second_flush_started.lock() = true;
            *second_flush_result.lock() = Some(page_cache.read().flush_range(0..PAGE_SIZE));
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

/// Anonymous reads span batches, preserve holes, and stop at the VMO boundary.
#[ktest]
fn anonymous_read_spans_batches_and_clamps_to_size() {
    let size = 35 * PAGE_SIZE;
    let page_cache = PageCache::new_anon(size).unwrap();
    let data = [0x5a; 17];
    page_cache.write_bytes(PAGE_SIZE - 7, &data).unwrap();
    let mut expected = vec![0; size];
    expected[PAGE_SIZE - 7..PAGE_SIZE - 7 + data.len()].copy_from_slice(&data);

    let offset = 13;
    let mut buf = vec![0xff; size + PAGE_SIZE];
    let mut writer = VmWriter::from(&mut buf[..]).to_fallible();
    let readahead = Readahead::default();
    page_cache
        .read_buffered(offset, &mut writer, Some(&readahead))
        .unwrap();
    assert_eq!(writer.avail(), PAGE_SIZE + offset);
    assert_eq!(&buf[..size - offset], &expected[offset..]);
    assert!(buf[size - offset..].iter().all(|&byte| byte == 0xff));
}

/// An uninitialized page stops collection before a later readahead marker.
#[ktest]
fn buffered_readahead_preserves_marker_after_uninitialized_page() {
    let backend = MockPageCacheBackend::new(16);
    backend.set_completion(IoKind::Read, IoCompletion::Deferred);
    backend.set_persisted_page_bytes(0, &vec![0x5a; PAGE_SIZE]);
    let page_cache = new_backend_page_cache(&backend, 16);
    let vmo = page_cache.as_vmo();
    vmo.readahead(1..2, None).unwrap();
    assert!(backend.complete_next_deferred_bio(IoKind::Read, false));
    backend.set_completion(IoKind::Read, IoCompletion::Immediate);
    vmo.readahead(0..4, Some(2)).unwrap();
    backend.fail_read_submission(1);

    let readahead = Readahead::default();
    let mut buf = vec![0xff; 3 * PAGE_SIZE];
    let mut writer = VmWriter::from(&mut buf[..]).to_fallible();
    assert_eq!(
        page_cache.read_buffered(0, &mut writer, Some(&readahead)),
        Err(ostd::Error::IoError)
    );
    assert_eq!(writer.avail(), 2 * PAGE_SIZE);
    assert!(buf[..PAGE_SIZE].iter().all(|&byte| byte == 0x5a));
    assert!(buf[PAGE_SIZE..].iter().all(|&byte| byte == 0xff));
    assert_eq!(backend.read_count(1), 2);
    assert_eq!(backend.read_count(4), 0);
    assert!(matches!(
        vmo.take_readahead_trigger(2),
        Some(ReadaheadTrigger::Marker)
    ));
}

/// A marker inserted during a cold read must not wait behind the rest of
/// the demand batch. Regression for the 2026-10-28 readahead review.
#[ktest]
fn buffered_readahead_continues_before_later_pages_complete() {
    let backend = MockPageCacheBackend::new(96);
    backend.set_completion(IoKind::Read, IoCompletion::Deferred);
    let page_cache = Arc::new(new_backend_page_cache(&backend, 96));
    let read_thread = {
        let page_cache = page_cache.clone();
        ThreadOptions::new(move || {
            let readahead = Readahead::default();
            let mut buf = vec![0; 64 * PAGE_SIZE];
            let mut writer = VmWriter::from(&mut buf[..]).to_fallible();
            page_cache
                .read_buffered(0, &mut writer, Some(&readahead))
                .unwrap();
            assert_eq!(writer.avail(), 0);
        })
        .spawn()
    };

    backend.wait_for_deferred_bios(IoKind::Read, 32);
    // The initial window's marker is page 16. Hold pages 17..31 pending.
    for _ in 0..17 {
        assert!(backend.complete_next_deferred_bio(IoKind::Read, true));
    }
    let continued = try_wait_until(|| backend.read_count(32) > 0);

    // Release the reader even if the continuation did not happen in time.
    backend.set_completion(IoKind::Read, IoCompletion::Immediate);
    while backend.complete_next_deferred_bio(IoKind::Read, true) {}
    read_thread.join();
    assert!(continued, "readahead waited for pages after its marker");
    // The same read must also reach the next marker and prefetch beyond
    // its requested range without rereading any page.
    for page_idx in 0..96 {
        assert_eq!(backend.read_count(page_idx), 1);
    }
}

/// A random request must not inflate the initial window of a new stream.
/// Regression for the 2026-10-28 readahead review.
#[ktest]
fn buffered_readahead_initializes_after_a_random_read() {
    let backend = MockPageCacheBackend::new(160);
    let page_cache = new_backend_page_cache(&backend, 160);
    let readahead = Readahead::default();
    let mut buf = vec![0; 16 * PAGE_SIZE];
    let mut writer = VmWriter::from(&mut buf[..]).to_fallible();
    page_cache
        .read_buffered(100 * PAGE_SIZE, &mut writer, Some(&readahead))
        .unwrap();
    readahead.finish_read(100 * PAGE_SIZE, buf.len());
    for page_idx in 100..116 {
        assert_eq!(backend.read_count(page_idx), 1);
        assert!(
            page_cache
                .as_vmo()
                .take_readahead_trigger(page_idx)
                .is_none()
        );
    }
    assert_eq!(backend.read_count(116), 0);

    let mut page = [0; PAGE_SIZE];
    let mut writer = VmWriter::from(&mut page[..]).to_fallible();
    page_cache
        .read_buffered(116 * PAGE_SIZE, &mut writer, Some(&readahead))
        .unwrap();
    for page_idx in 116..120 {
        assert_eq!(backend.read_count(page_idx), 1);
    }
    assert_eq!(backend.read_count(120), 0);
}

/// A cached prefix must not hide the first missing page from read readahead.
#[ktest]
fn buffered_readahead_starts_after_cached_prefix() {
    let backend = MockPageCacheBackend::new(8);
    let page_cache = new_backend_page_cache(&backend, 8);
    let readahead = Readahead::default();
    page_cache.write_bytes(0, &[0; PAGE_SIZE]).unwrap();

    let mut buf = vec![0; 2 * PAGE_SIZE];
    let mut writer = VmWriter::from(&mut buf[..]).to_fallible();
    page_cache
        .read_buffered(0, &mut writer, Some(&readahead))
        .unwrap();

    assert_eq!(backend.read_count(0), 0);
    for page_idx in 1..5 {
        assert_eq!(backend.read_count(page_idx), 1);
    }
    assert_eq!(backend.read_count(5), 0);
}

/// Readahead clamps to the VMO size and skips cached gaps without changing
/// the backend indices of missing runs or marking resident pages.
#[ktest]
fn readahead_preserves_indices_across_cached_gaps() {
    let backend = MockPageCacheBackend::new(8);
    let page_cache = new_backend_page_cache(&backend, 8);
    let resident = [1, 4, 7];
    for page_idx in 0..8 {
        let pattern = vec![page_idx as u8; PAGE_SIZE];
        backend.set_persisted_page_bytes(page_idx, &pattern);
        if resident.contains(&page_idx) {
            page_cache
                .write_bytes(page_idx * PAGE_SIZE, &pattern)
                .unwrap();
        }
    }

    let vmo = page_cache.as_vmo();
    vmo.readahead(0..12, Some(4)).unwrap();
    assert!(vmo.take_readahead_trigger(4).is_none());
    for page_idx in 0..8 {
        let mut page = [0; PAGE_SIZE];
        page_cache
            .read_bytes(page_idx * PAGE_SIZE, &mut page)
            .unwrap();
        assert!(page.iter().all(|&byte| byte == page_idx as u8));
        assert_eq!(
            backend.read_count(page_idx),
            usize::from(!resident.contains(&page_idx))
        );
    }
}

/// A demand read succeeds even after a readahead I/O completes with failure.
#[ktest]
fn readahead_io_failure_allows_demand_retry() {
    let backend = MockPageCacheBackend::new(2);
    backend.set_completion(IoKind::Read, IoCompletion::Deferred);

    let page_cache = new_backend_page_cache(&backend, 2);
    page_cache.as_vmo().readahead(0..1, None).unwrap();

    backend.wait_for_deferred_bios(IoKind::Read, 1);
    // Complete the readahead with an I/O error.
    backend.complete_next_deferred_bio(IoKind::Read, false);

    // Switch to immediate completion for the demand retry.
    backend.set_completion(IoKind::Read, IoCompletion::Immediate);
    let pattern = vec![0xCC; PAGE_SIZE];
    backend.set_persisted_page_bytes(0, &pattern);

    let mut buf = vec![0u8; PAGE_SIZE];
    page_cache.read_bytes(0, &mut buf).unwrap();

    // The demand read should have triggered a new backend read.
    assert_eq!(backend.read_count(0), 2);
    assert_eq!(buf, pattern);
}

/// A mapping can continue a marked buffered-read readahead window.
#[ktest]
fn readahead_marker_continues_buffered_read_window() {
    let backend = MockPageCacheBackend::new(20);
    let page_cache = new_backend_page_cache(&backend, 20);
    let vmo = page_cache.as_vmo();
    let readahead = Readahead::default();
    readahead.on_read_miss(vmo, 0..1, 0);
    page_cache.read_bytes(0, &mut [0; PAGE_SIZE]).unwrap();
    readahead.finish_read(0, PAGE_SIZE);
    for page_idx in 0..4 {
        assert_eq!(backend.read_count(page_idx), 1);
    }

    readahead.on_fault(vmo, 2);
    assert_eq!(backend.read_count(4), 0);
    readahead.on_fault(vmo, 1);
    for page_idx in 4..12 {
        assert_eq!(backend.read_count(page_idx), 1);
    }
    assert_eq!(backend.read_count(12), 0);

    readahead.on_fault(vmo, 1);
    assert_eq!(backend.read_count(12), 0);
    // The next marker advances the window, clamped to the cache capacity.
    readahead.on_fault(vmo, 4);
    for page_idx in 0..20 {
        assert_eq!(backend.read_count(page_idx), 1);
    }
    assert!(matches!(
        vmo.take_readahead_trigger(12),
        Some(ReadaheadTrigger::Marker)
    ));
}

/// An independent open infers a new window from another open's cached marker.
#[ktest]
fn readahead_marker_shared_between_independent_opens() {
    let backend = MockPageCacheBackend::new(40);
    let page_cache = new_backend_page_cache(&backend, 40);
    let vmo = page_cache.as_vmo();
    let first_readahead = Readahead::default();
    let second_readahead = Readahead::default();

    first_readahead.on_fault(vmo, 0);
    for page_idx in 0..32 {
        assert_eq!(backend.read_count(page_idx), 1);
    }
    assert_eq!(backend.read_count(32), 0);

    second_readahead.on_fault(vmo, 24);
    for page_idx in 0..40 {
        assert_eq!(backend.read_count(page_idx), 1);
    }
    assert!(vmo.take_readahead_trigger(24).is_none());
    assert!(matches!(
        vmo.take_readahead_trigger(32),
        Some(ReadaheadTrigger::Marker)
    ));
}
