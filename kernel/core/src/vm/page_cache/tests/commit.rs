// SPDX-License-Identifier: MPL-2.0

//! Cold-page initialization and error propagation through VMO commit interfaces.
//!
//! These tests exercise commit/probe operations without actual userspace faults
//! or page-table mappings.

use alloc::vec;

use ostd::prelude::ktest;

use super::base::{self, IoCompletion, IoKind, MockPageCacheBackend};
use crate::{
    prelude::*,
    thread::kernel_thread::ThreadOptions,
    vm::page_cache::{VmoCommitError, VmoMapMode},
};

/// Commits the same cold page through `try_commit_page()` and `commit_on()`,
/// verifying that one backend read initializes the page for all waiters.
#[ktest]
fn shares_cold_read() {
    let backend = MockPageCacheBackend::new(1);
    backend.set_completion(IoKind::Read, IoCompletion::Deferred);
    backend.set_persisted_page_bytes(0, &[0x6b; PAGE_SIZE]);

    let page_cache = base::new_backend_page_cache(&backend, 1);
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
        other => panic!("unexpected commit state: {other:?}"),
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
    vmo.read(0, &mut writer).unwrap();
    assert_eq!(read_buffer, vec![0x6b; PAGE_SIZE]);
}

/// Keeps backend reads failing for one page and checks both the initial
/// committer and a later waiter return `EIO` instead of livelocking.
#[ktest]
fn returns_read_errors() {
    let backend = MockPageCacheBackend::new(1);
    backend.set_completion(IoKind::Read, IoCompletion::Deferred);

    let page_cache = base::new_backend_page_cache(&backend, 1);
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
