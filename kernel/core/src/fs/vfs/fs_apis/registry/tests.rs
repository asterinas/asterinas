// SPDX-License-Identifier: MPL-2.0

//! Tests cache reuse and replacement with controlled final sync, without disk I/O.

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ostd::{prelude::ktest, sync::WaitQueue};

use super::*;
use crate::{
    fs::{
        ramfs::RamFs,
        vfs::{
            file_system::{FileSystem, FsEventSubscriberStats, SuperBlock},
            fs_handle::FsLifecycle,
            inode::Inode,
            path::{Mount, PerMountFlags},
        },
    },
    thread::{Thread, kernel_thread::ThreadOptions},
    time::clocks,
};

const MAX_WAIT_YIELDS: usize = 1_000_000;

/// A pause inside final sync that the test thread controls.
struct SyncGate {
    has_started: AtomicBool,
    is_released: AtomicBool,
    // The sync body has finished, but filesystem teardown may still be pending.
    has_finished: AtomicBool,
    wait_queue: WaitQueue,
    result: Result<()>,
}

impl SyncGate {
    fn new(result: Result<()>) -> Arc<Self> {
        Arc::new(Self {
            has_started: AtomicBool::new(false),
            is_released: AtomicBool::new(false),
            has_finished: AtomicBool::new(false),
            wait_queue: WaitQueue::new(),
            result,
        })
    }

    fn sync(&self) -> Result<()> {
        self.has_started.store(true, Ordering::Release);
        self.wait_queue.wake_all();
        self.wait_queue
            .wait_until(|| self.is_released.load(Ordering::Acquire).then_some(()));
        self.has_finished.store(true, Ordering::Release);
        self.result
    }

    fn release(&self) {
        self.is_released.store(true, Ordering::Release);
        self.wait_queue.wake_all();
    }
}

/// A mock filesystem with its own lifecycle, flags, and sync behavior.
/// `RamFs` only supplies the root inode and metadata needed by the cache tests.
struct TestFs {
    lifecycle: FsLifecycle,
    ramfs: FsHandle<RamFs>,
    flags: FsFlags,
    sync_gate: Option<Arc<SyncGate>>,
}

impl FileSystem for TestFs {
    fn lifecycle(&self) -> &FsLifecycle {
        &self.lifecycle
    }

    fn name(&self) -> &'static str {
        "cache-test"
    }

    fn sync(&self) -> Result<()> {
        self.sync_gate.as_ref().map_or(Ok(()), |gate| gate.sync())
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.ramfs.root_inode()
    }

    fn sb(&self) -> SuperBlock {
        self.ramfs.sb()
    }

    fn flags(&self) -> FsFlags {
        self.flags
    }

    fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        self.ramfs.fs_event_subscriber_stats()
    }
}

fn new_fs(flags: FsFlags, sync_gate: Option<Arc<SyncGate>>) -> FsHandle {
    crate::thread::work_queue::init_for_ktest();
    FsHandle::new(Arc::new(TestFs {
        lifecycle: FsLifecycle::new(),
        ramfs: RamFs::new(),
        flags,
        sync_gate,
    }))
    .into_dyn()
}

// Return on timeout so the caller can unblock and join workers before asserting.
fn wait_until(mut condition_fn: impl FnMut() -> bool) -> bool {
    for _ in 0..MAX_WAIT_YIELDS {
        if condition_fn() {
            return true;
        }
        Thread::yield_now();
    }
    false
}

fn check_recreation_waits(
    sync_result: Result<()>,
    release_fn: impl FnOnce(FsAndRoot) + Send + 'static,
) {
    clocks::init_for_ktest();
    const KEY: u32 = 1;
    const NUM_WAITERS: usize = 2;

    // Retire the old instance, but hold final sync so replacements must wait.
    let cache = Arc::new(FsCache::new());
    let sync_gate = SyncGate::new(sync_result);
    let old_fs_and_root = cache
        .get_or_create(KEY, FsFlags::empty(), || {
            Ok(new_fs(FsFlags::empty(), Some(sync_gate.clone())))
        })
        .unwrap();

    // Keeping the allocation alive must not make the retired instance reusable.
    let old_fs_memory = old_fs_and_root.fs().as_arc().clone();
    let old_fs_weak = old_fs_and_root.fs().downgrade();
    let completion = old_fs_and_root.fs().shutdown_completion().clone();
    let drop_thread = ThreadOptions::new(move || release_fn(old_fs_and_root)).spawn();
    sync_gate
        .wait_queue
        .wait_until(|| sync_gate.has_started.load(Ordering::Acquire).then_some(()));
    assert!(
        old_fs_weak.upgrade().is_none(),
        "retired filesystem still accepts active handles"
    );

    // The cache, retained allocation, shutdown, and local completion references
    // stay alive while sync is blocked. Each waiting lookup adds one more.
    let completion_refs_with_waiters = Arc::strong_count(&completion) + NUM_WAITERS;

    // Two callers request the same key: successful sync must yield one replacement.
    let num_replacements = Arc::new(AtomicUsize::new(0));
    let has_early_replacement = Arc::new(AtomicBool::new(false));
    let num_completed_lookups = Arc::new(AtomicUsize::new(0));
    let lookup_results = Arc::new(Mutex::new(Vec::new()));
    let mut lookup_threads = Vec::new();
    for _ in 0..NUM_WAITERS {
        let cache = cache.clone();
        let sync_gate = sync_gate.clone();
        let num_replacements = num_replacements.clone();
        let has_early_replacement = has_early_replacement.clone();
        let num_completed_lookups = num_completed_lookups.clone();
        let lookup_results = lookup_results.clone();
        lookup_threads.push(
            ThreadOptions::new(move || {
                let result = cache.get_or_create(KEY, FsFlags::empty(), || {
                    num_replacements.fetch_add(1, Ordering::Relaxed);
                    if !sync_gate.has_finished.load(Ordering::Acquire) {
                        has_early_replacement.store(true, Ordering::Release);
                    }
                    Ok(new_fs(FsFlags::empty(), None))
                });
                lookup_results.lock().push(result);
                num_completed_lookups.fetch_add(1, Ordering::Release);
            })
            .spawn(),
        );
    }

    // `get_or_create` clones the completion before unlocking and waiting. Observe
    // both callers entering that path; check lock availability separately below.
    // Early returns also end the wait, but fail the assertions after cleanup.
    let lookup_state_observed = wait_until(|| {
        Arc::strong_count(&completion) >= completion_refs_with_waiters
            || num_completed_lookups.load(Ordering::Acquire) == NUM_WAITERS
    });
    let all_lookups_waiting = num_completed_lookups.load(Ordering::Acquire) == 0
        && Arc::strong_count(&completion) >= completion_refs_with_waiters;

    // Waiting for this key must leave the cache usable for unrelated keys.
    let cache_lock_available =
        all_lookups_waiting && wait_until(|| cache.entries.try_lock().is_some());
    let other_key_fs = if cache_lock_available {
        Some(
            cache
                .get_or_create(KEY + 1, FsFlags::empty(), || {
                    Ok(new_fs(FsFlags::empty(), None))
                })
                .unwrap(),
        )
    } else {
        None
    };

    // Release sync and join before checking the observations above, so failed
    // wait assertions do not leave these workers blocked in the test runner.
    sync_gate.release();
    drop_thread.join();
    for thread in lookup_threads {
        thread.join();
    }

    assert!(
        !has_early_replacement.load(Ordering::Acquire),
        "filesystem recreated before final writeback completed"
    );
    assert!(
        lookup_state_observed,
        "timed out observing replacement lookups"
    );
    assert!(all_lookups_waiting, "lookups did not wait for final sync");
    assert!(
        cache_lock_available,
        "cache lock remained held during final sync"
    );
    assert!(
        other_key_fs.is_some(),
        "an unrelated cache key could not be created"
    );
    assert!(
        FsHandle::try_acquire(old_fs_memory.clone()).is_none(),
        "cleanup allowed the old filesystem to become active again"
    );

    let lookup_results = lookup_results.lock();
    match sync_result {
        Ok(()) => {
            assert_eq!(
                num_replacements.load(Ordering::Relaxed),
                1,
                "concurrent lookups must create exactly one replacement"
            );
            assert!(
                Arc::ptr_eq(
                    lookup_results[0].as_ref().unwrap().fs().as_arc(),
                    lookup_results[1].as_ref().unwrap().fs().as_arc(),
                ),
                "concurrent lookups returned different filesystem instances"
            );
        }
        Err(err) => {
            assert_eq!(
                num_replacements.load(Ordering::Relaxed),
                0,
                "failed final sync must prevent replacement creation"
            );
            for result in lookup_results.iter() {
                assert_eq!(result.as_ref().unwrap_err().error(), err.error());
            }
            assert_eq!(
                cache
                    .get_or_create(KEY, FsFlags::empty(), || panic!(
                        "failed final sync must prevent recreation"
                    ))
                    .unwrap_err()
                    .error(),
                err.error()
            );
        }
    }
    assert_eq!(
        completion.wait().map_err(|err| err.error()),
        sync_result.map_err(|err| err.error())
    );
}

#[ktest]
fn cache_recreation_waits_for_final_sync() {
    check_recreation_waits(Ok(()), drop);
}

#[ktest]
fn cache_recreation_reports_final_sync_error() {
    check_recreation_waits(Err(Error::new(Errno::EIO)), drop);
}

#[ktest]
fn cache_recreation_waits_for_synchronous_drop() {
    check_recreation_waits(Ok(()), |fs_and_root| {
        let (fs, root) = fs_and_root.into_parts();
        drop(root);
        fs.drop_synchronously().unwrap();
    });
}

#[ktest]
fn cache_recreation_reports_synchronous_drop_error() {
    check_recreation_waits(Err(Error::new(Errno::EIO)), |fs_and_root| {
        let (fs, root) = fs_and_root.into_parts();
        drop(root);
        let completion = fs.shutdown_completion().clone();
        assert_eq!(fs.drop_synchronously().unwrap_err().error(), Errno::EIO);
        assert!(matches!(completion.result(), Some(Err(err)) if err.error() == Errno::EIO));
    });
}

#[ktest]
fn cache_reuses_live_filesystems_and_checks_flags() {
    clocks::init_for_ktest();
    let cache = FsCache::new();
    let fs = cache
        .get_or_create(1u32, FsFlags::RDONLY, || Ok(new_fs(FsFlags::RDONLY, None)))
        .unwrap();
    let reused = cache
        .get_or_create(1, FsFlags::RDONLY, || {
            panic!("live filesystem must be reused")
        })
        .unwrap();
    assert!(Arc::ptr_eq(fs.fs().as_arc(), reused.fs().as_arc()));
    assert!(Arc::ptr_eq(&fs.root_dentry, &reused.root_dentry));
    assert_eq!(
        cache
            .get_or_create(1, FsFlags::empty(), || panic!(
                "mismatched live filesystem must not be replaced"
            ))
            .unwrap_err()
            .error(),
        Errno::EBUSY
    );
}

#[ktest]
fn cache_creation_failure_releases_the_key() {
    clocks::init_for_ktest();
    let cache = FsCache::new();
    let fs = cache
        .get_or_create(1u32, FsFlags::empty(), || {
            Ok(new_fs(FsFlags::empty(), None))
        })
        .unwrap();
    drop(fs);

    // A factory failure is retryable, unlike a failed final sync of the old instance.
    assert_eq!(
        cache
            .get_or_create(1, FsFlags::empty(), || Err(Error::new(Errno::EIO)))
            .unwrap_err()
            .error(),
        Errno::EIO
    );
    assert!(!cache.entries.lock().contains_key(&1));
    assert!(
        cache
            .get_or_create(1, FsFlags::empty(), || Ok(new_fs(FsFlags::empty(), None)))
            .is_ok()
    );
}

#[ktest]
fn cache_recreates_expired_roots_without_replacing_the_filesystem() {
    clocks::init_for_ktest();
    let cache = FsCache::new();
    let original = cache
        .get_or_create(1u32, FsFlags::empty(), || {
            Ok(new_fs(FsFlags::empty(), None))
        })
        .unwrap();

    // Keep the filesystem active while its root expires: only the dentry should change.
    let fs = original.fs().clone();
    let completion = fs.shutdown_completion().clone();
    let old_root = Arc::downgrade(&original.root_dentry);
    drop(original);
    assert!(old_root.upgrade().is_none());

    let recreated = cache
        .get_or_create(1, FsFlags::empty(), || {
            panic!("the filesystem is still active")
        })
        .unwrap();
    let shared = cache
        .get_or_create(1, FsFlags::empty(), || {
            panic!("the filesystem is still active")
        })
        .unwrap();
    assert!(Arc::ptr_eq(fs.as_arc(), recreated.fs().as_arc()));
    assert!(Arc::ptr_eq(&recreated.root_dentry, &shared.root_dentry));
    drop(shared);
    drop(recreated);
    drop(fs);
    completion.wait().unwrap();
}

#[ktest]
fn detached_mount_retains_selected_root() {
    clocks::init_for_ktest();
    let fs_and_root = FsAndRoot::new(new_fs(FsFlags::empty(), None));
    let root = fs_and_root.root_dentry.clone();
    let completion = fs_and_root.fs().shutdown_completion().clone();
    let mount =
        Mount::new_detached(fs_and_root, PerMountFlags::empty(), Weak::new(), None).unwrap();
    assert!(Arc::ptr_eq(mount.root_dentry(), &root));
    drop(root);
    drop(mount);
    completion.wait().unwrap();
}

#[ktest]
fn cache_tracks_volatile_filesystems_too() {
    clocks::init_for_ktest();
    crate::thread::work_queue::init_for_ktest();
    let cache = FsCache::new();
    let fs = cache
        .get_or_create(1u32, FsFlags::empty(), || Ok(RamFs::new().into_dyn()))
        .unwrap();
    let completion = fs.fs().shutdown_completion().clone();
    drop(fs);
    completion.wait().unwrap();
    assert!(
        cache
            .get_or_create(1, FsFlags::empty(), || Ok(RamFs::new().into_dyn()))
            .is_ok()
    );
}
