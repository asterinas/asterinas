// SPDX-License-Identifier: MPL-2.0

//! Tests for writing cached data to the backend with `PageCache::flush_range`.
//!
//! These cover pages changed during a flush, flushes running at the same time,
//! and writes that the backend rejects or fails to complete successfully.

use alloc::vec;

use ostd::{mm::VmIo, prelude::ktest};

use super::base::{self, IoCompletion, IoKind, MockPageCacheBackend, WriteSubmission};
use crate::{
    prelude::*,
    thread::kernel_thread::ThreadOptions,
    vm::page_cache::{CachePageExt, PageCache, VmoMapMode},
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

/// Checks that a flush notices a page marked dirty again while it waits for the page lock.
#[ktest]
fn rechecks_after_lock() {
    let backend = MockPageCacheBackend::new(1);
    let page_cache = Arc::new(base::new_backend_page_cache(&backend, 1));
    let pattern = vec![0xb9; PAGE_SIZE];
    page_cache.write_bytes(0, &pattern).unwrap();
    let (page, _) = page_cache
        .as_vmo()
        .try_commit_page(0, VmoMapMode::SharedRead)
        .unwrap();

    // Simulate a submitter holding a page it has just marked clean.
    let mut prepared_page = page.clone().lock();
    prepared_page.prepare_writeback();
    let result = Arc::new(Mutex::new(None::<Result<()>>));
    let flush_thread = {
        let page_cache = page_cache.clone();
        let result = result.clone();
        ThreadOptions::new(move || {
            *result.lock() = Some(page_cache.flush_range(0..PAGE_SIZE));
        })
        .spawn()
    };

    // Wake the flush while the page is still locked. It must keep waiting
    // until the submitter drops the page and restores dirty.
    base::wait_until(
        || page.wait_queue().wake_all() > 0,
        "timed out waiting to wake the flush while page 0 is prepared",
    );
    assert!(result.lock().is_none());
    drop(prepared_page);

    flush_thread.join();
    assert!(result.lock().take().unwrap().is_ok());
    assert_eq!(backend.write_count(0), 1);
    assert_eq!(backend.persisted_page_bytes(0), pattern);
}

/// Tests for a flush whose first write is rejected by the backend.
///
/// The flush must stop submitting writes and leave all dirty pages ready for another flush.
/// Once the backend accepts writes again, the next flush must write all of them.
mod first_rejection {
    use super::*;

    #[ktest]
    fn per_page_sparse() {
        run(WriteSubmission::PerPage, &[0, 2]);
    }

    #[ktest]
    fn per_page_multiple_runs() {
        run(WriteSubmission::PerPage, &[0, 1, 3, 4]);
    }

    #[ktest]
    fn contiguous_sparse() {
        run(WriteSubmission::Contiguous, &[0, 2]);
    }

    #[ktest]
    fn contiguous_multiple_runs() {
        run(WriteSubmission::Contiguous, &[0, 1, 3, 4]);
    }

    fn run(submission: WriteSubmission, dirty_pages: &[usize]) {
        let backend = MockPageCacheBackend::new(5);
        backend.set_write_submission(submission);
        let page_cache = base::new_backend_page_cache(&backend, 5);
        let pattern = vec![0x71; PAGE_SIZE];
        for &idx in dirty_pages {
            page_cache.write_bytes(idx * PAGE_SIZE, &pattern).unwrap();
        }
        backend.set_write_submit_failure(0, true);

        assert_eq!(
            page_cache
                .flush_range(0..5 * PAGE_SIZE)
                .unwrap_err()
                .error(),
            Errno::EBUSY
        );
        for &idx in &dirty_pages[1..] {
            assert_eq!(
                backend.write_count(idx),
                0,
                "{submission:?}, dirty pages {dirty_pages:?}, page {idx}"
            );
        }

        for &idx in dirty_pages {
            assert_rolled_back(&page_cache, idx);
        }
        backend.set_write_submit_failure(0, false);
        page_cache.flush_range(0..5 * PAGE_SIZE).unwrap();
        for &idx in dirty_pages {
            assert_eq!(
                backend.persisted_page_bytes(idx),
                pattern,
                "{submission:?}, dirty pages {dirty_pages:?}, page {idx}"
            );
            assert_eq!(backend.write_count(idx), if idx == 0 { 2 } else { 1 });
        }
    }
}

/// Tests for a flush that writes one page before the backend rejects the next write.
///
/// The rejected page and the pages not yet submitted must stay dirty for the next flush.
/// The page already written should not be written again.
mod partial_rejection {
    use super::*;

    #[ktest]
    fn per_page() {
        run(WriteSubmission::PerPage);
    }

    #[ktest]
    fn contiguous() {
        run(WriteSubmission::Contiguous);
    }

    fn run(submission: WriteSubmission) {
        let backend = MockPageCacheBackend::new(6);
        backend.set_write_submission(submission);
        let page_cache = base::new_backend_page_cache(&backend, 6);
        let pattern = vec![0x83; PAGE_SIZE];
        for idx in [0, 1, 2, 4, 5] {
            page_cache.write_bytes(idx * PAGE_SIZE, &pattern).unwrap();
        }
        backend.set_write_submit_failure(1, true);

        assert_eq!(
            page_cache
                .flush_range(0..6 * PAGE_SIZE)
                .unwrap_err()
                .error(),
            Errno::EBUSY
        );
        assert_eq!(backend.persisted_page_bytes(0), pattern);
        for idx in [2, 4, 5] {
            assert_eq!(backend.write_count(idx), 0, "{submission:?}, page {idx}");
        }

        for idx in [1, 2, 4, 5] {
            assert_rolled_back(&page_cache, idx);
        }
        backend.set_write_submit_failure(1, false);
        page_cache.flush_range(0..6 * PAGE_SIZE).unwrap();
        for idx in [0, 1, 2, 4, 5] {
            assert_eq!(
                backend.persisted_page_bytes(idx),
                pattern,
                "{submission:?}, page {idx}"
            );
            assert_eq!(backend.write_count(idx), if idx == 1 { 2 } else { 1 });
        }
    }
}

/// Tests for a flush whose later write is rejected while an earlier write is still in progress.
///
/// The flush must wait for the earlier write to finish before returning the rejection error.
/// Even if that earlier write also fails, the flush must still report the rejection error.
mod drains_accepted_io {
    use super::*;

    #[ktest]
    fn separate_runs() {
        run(&[0, 2, 4]);
    }

    #[ktest]
    fn same_run() {
        run(&[0, 1, 2, 4]);
    }

    fn run(dirty_pages: &[usize]) {
        let rejected_idx = dirty_pages[1];
        let backend = MockPageCacheBackend::new(5);
        backend.set_completion(IoKind::Write, IoCompletion::Deferred);
        backend.set_write_submit_failure(rejected_idx, true);
        let page_cache = Arc::new(base::new_backend_page_cache(&backend, 5));
        for &idx in dirty_pages {
            page_cache
                .write_bytes(idx * PAGE_SIZE, &vec![0x95; PAGE_SIZE])
                .unwrap();
        }
        let result = Arc::new(Mutex::new(None::<Result<()>>));
        let flush_thread = {
            let page_cache = page_cache.clone();
            let result = result.clone();
            ThreadOptions::new(move || {
                *result.lock() = Some(page_cache.flush_range(0..5 * PAGE_SIZE));
            })
            .spawn()
        };

        // Check that flush is still running after it starts waiting for page 0.
        base::wait_until(
            || backend.write_wait_count(0) > 0 || result.lock().is_some(),
            "timed out waiting for the rejected flush to drain page 0 I/O",
        );
        assert!(result.lock().is_none());
        assert_eq!(backend.write_count(rejected_idx), 1);
        assert!(backend.complete_next_deferred_bio(IoKind::Write, false));
        flush_thread.join();
        assert_eq!(
            result.lock().take().unwrap().unwrap_err().error(),
            Errno::EBUSY
        );
        assert!(!backend.complete_next_deferred_bio(IoKind::Write, true));
    }
}

/// Tests for a flush that fails while a previous flush still has a write in progress.
///
/// The failing flush must wait for that write before returning, even if the page was changed again.
/// These cases cover both a rejected write and a write that is accepted but later fails.
mod drains_previous_on_error {
    use super::*;

    #[derive(Clone, Copy, Debug)]
    enum OldPageState {
        Clean,
        Redirtied,
    }

    #[derive(Clone, Copy, Debug)]
    enum FailureStage {
        Submission,
        Completion,
    }

    #[ktest]
    fn clean_submit_rejected() {
        run(OldPageState::Clean, FailureStage::Submission);
    }

    #[ktest]
    fn redirtied_submit_rejected() {
        run(OldPageState::Redirtied, FailureStage::Submission);
    }

    #[ktest]
    fn clean_completion_failed() {
        run(OldPageState::Clean, FailureStage::Completion);
    }

    #[ktest]
    fn redirtied_completion_failed() {
        run(OldPageState::Redirtied, FailureStage::Completion);
    }

    fn run(old_state: OldPageState, failure: FailureStage) {
        let backend = MockPageCacheBackend::new(3);
        backend.set_completion(IoKind::Write, IoCompletion::Deferred);
        let page_cache = Arc::new(base::new_backend_page_cache(&backend, 3));
        let pattern = vec![0xa7; PAGE_SIZE];
        page_cache.write_bytes(2 * PAGE_SIZE, &pattern).unwrap();
        let (old_page, _) = page_cache
            .as_vmo()
            .try_commit_page(2 * PAGE_SIZE, VmoMapMode::SharedRead)
            .unwrap();
        let first_result = Arc::new(Mutex::new(None::<Result<()>>));
        let first_flush = {
            let page_cache = page_cache.clone();
            let first_result = first_result.clone();
            ThreadOptions::new(move || {
                *first_result.lock() = Some(page_cache.flush_range(2 * PAGE_SIZE..3 * PAGE_SIZE));
            })
            .spawn()
        };
        backend.wait_for_deferred_bios(IoKind::Write, 1);
        // The first flush releases the page lock before waiting for I/O.
        base::wait_until(
            || backend.write_wait_count(2) > 0,
            "timed out waiting for the first flush to wait on page 2 I/O",
        );
        if matches!(old_state, OldPageState::Redirtied) {
            page_cache.write_bytes(2 * PAGE_SIZE, &pattern).unwrap();
        }
        page_cache.write_bytes(0, &pattern).unwrap();
        backend.set_write_submit_failure(0, matches!(failure, FailureStage::Submission));
        let second_result = Arc::new(Mutex::new(None::<Result<()>>));
        let second_flush = {
            let page_cache = page_cache.clone();
            let second_result = second_result.clone();
            ThreadOptions::new(move || {
                *second_result.lock() = Some(page_cache.flush_range(0..3 * PAGE_SIZE));
            })
            .spawn()
        };

        if matches!(failure, FailureStage::Completion) {
            // Fail the newer write first, leaving the older one pending.
            backend.wait_for_deferred_bios(IoKind::Write, 2);
            assert!(backend.complete_last_deferred_bio(IoKind::Write, false));
        }
        base::wait_until(
            || {
                second_result.lock().is_some()
                    || (backend.write_count(0) > 0
                        && (matches!(old_state, OldPageState::Clean)
                            || matches!(failure, FailureStage::Completion)
                            || old_page.state().is_dirty())
                        && old_page.try_lock_guard().is_none())
            },
            "timed out waiting for the failing flush to wait on page 2 writeback",
        );
        assert!(second_result.lock().is_none(), "{old_state:?}, {failure:?}");
        assert!(backend.complete_next_deferred_bio(IoKind::Write, true));
        if matches!(old_state, OldPageState::Redirtied)
            && matches!(failure, FailureStage::Completion)
        {
            // The second flush must also write the page dirtied again.
            backend.wait_for_deferred_bios(IoKind::Write, 1);
            assert!(backend.complete_next_deferred_bio(IoKind::Write, true));
        }
        first_flush.join();
        second_flush.join();
        assert!(first_result.lock().take().unwrap().is_ok());
        assert_eq!(
            second_result.lock().take().unwrap().unwrap_err().error(),
            if matches!(failure, FailureStage::Submission) {
                Errno::EBUSY
            } else {
                Errno::EIO
            },
            "{old_state:?}, {failure:?}"
        );
    }
}

/// Checks that a page whose write was not submitted is dirty, unlocked, and not being written back.
fn assert_rolled_back(page_cache: &PageCache, page_idx: usize) {
    let (page, _) = page_cache
        .as_vmo()
        .try_commit_page(page_idx * PAGE_SIZE, VmoMapMode::SharedRead)
        .unwrap();
    let locked_page = page
        .try_lock_guard()
        .expect("unsubmitted page is still locked");
    assert!(locked_page.state().is_dirty(), "page {page_idx}");
    assert!(!locked_page.is_writing_back(), "page {page_idx}");
}
