// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::AtomicBool;

use inherit_methods_macro::inherit_methods;
use ostd::{prelude::ktest, task::Task};

use super::*;
use crate::{
    fs::{
        ramfs::RamFs,
        vfs::{
            file_system::{FsEventSubscriberStats, SuperBlock},
            inode::Inode,
        },
    },
    thread::{Thread, kernel_thread::ThreadOptions},
};

struct Probe {
    lifecycle: FsLifecycle,
    ramfs: FsHandle<RamFs>,
    weak_self: Weak<Self>,
    sync_count: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
    releasing_task: Arc<Task>,
}

impl Probe {
    fn new() -> Arc<Self> {
        work_queue::init_for_ktest();
        Arc::new_cyclic(|weak| Self {
            lifecycle: FsLifecycle::new(),
            ramfs: RamFs::new(),
            weak_self: weak.clone(),
            sync_count: Arc::new(AtomicUsize::new(0)),
            dropped: Arc::new(AtomicBool::new(false)),
            releasing_task: Task::current().unwrap().cloned(),
        })
    }
}

#[inherit_methods(from = "self.ramfs")]
impl FileSystem for Probe {
    fn name(&self) -> &'static str;
    fn root_inode(&self) -> Arc<dyn Inode>;
    fn sb(&self) -> SuperBlock;
    fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats;
    fn lifecycle(&self) -> &FsLifecycle {
        &self.lifecycle
    }

    fn sync(&self) -> Result<()> {
        assert!(!Arc::ptr_eq(
            &self.releasing_task,
            &Task::current().unwrap().cloned()
        ));
        assert!(FsHandle::try_acquire(self.weak_self.upgrade().unwrap()).is_none());
        self.sync_count.fetch_add(1, Ordering::Release);
        Ok(())
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::Release);
    }
}

#[ktest]
fn activation_is_one_shot() {
    let memory = Probe::new();
    let count = memory.sync_count.clone();
    let dropped = memory.dropped.clone();
    assert!(FsHandle::try_acquire(memory.clone()).is_none());
    let owner = FsHandle::new(memory.clone());
    let completion = owner.shutdown_completion().clone();
    assert!(owner.downgrade().upgrade().is_some());
    drop(owner);
    completion.wait().unwrap();
    assert_eq!(count.load(Ordering::Acquire), 1);
    assert!(FsHandle::try_acquire(memory.clone()).is_none());
    assert!(!dropped.load(Ordering::Acquire));
    drop(memory);
    assert!(dropped.load(Ordering::Acquire));
}

#[ktest]
#[should_panic(expected = "filesystem was already activated")]
fn reactivating_active_filesystem_panics() {
    let memory = Probe::new();
    let _owner = FsHandle::new(memory.clone());
    let _ = FsHandle::new(memory);
}

#[ktest]
#[should_panic(expected = "filesystem was already activated")]
fn reactivating_retired_filesystem_panics() {
    let memory = Probe::new();
    let owner = FsHandle::new(memory.clone());
    let completion = owner.shutdown_completion().clone();
    drop(owner);
    completion.wait().unwrap();
    let _ = FsHandle::new(memory);
}

#[ktest]
fn final_release_under_spinlock_defers_sync() {
    let memory = Probe::new();
    let owner = FsHandle::new(memory.clone());
    let completion = owner.shutdown_completion().clone();
    let weak = owner.downgrade();
    let clone = owner.clone();
    drop(owner);
    assert!(Arc::ptr_eq(
        clone.as_arc(),
        weak.upgrade().unwrap().as_arc()
    ));
    assert!(completion.result().is_none());

    let lock: SpinLock<_> = SpinLock::new(());
    let guard = lock.lock();
    drop(clone);
    drop(guard);
    completion.wait().unwrap();
    assert!(weak.upgrade().is_none());
    assert_eq!(memory.sync_count.load(Ordering::Acquire), 1);
    assert!(!memory.dropped.load(Ordering::Acquire));
}

#[ktest]
fn weak_acquisition_races_with_final_release() {
    const NUM_THREADS: usize = 4;
    const NUM_ATTEMPTS: usize = 64;
    let memory = Probe::new();
    let owner = FsHandle::new(memory.clone());
    let completion = owner.shutdown_completion().clone();
    let ready = Arc::new(AtomicUsize::new(0));
    let start = Arc::new(AtomicBool::new(false));
    let queue = Arc::new(WaitQueue::new());
    let mut threads = Vec::new();
    for _ in 0..NUM_THREADS {
        let weak = owner.downgrade();
        let completion = completion.clone();
        let ready = ready.clone();
        let start = start.clone();
        let queue = queue.clone();
        threads.push(
            ThreadOptions::new(move || {
                ready.fetch_add(1, Ordering::Release);
                queue.wake_all();
                queue.wait_until(|| start.load(Ordering::Acquire).then_some(()));
                for _ in 0..NUM_ATTEMPTS {
                    if let Some(handle) = weak.upgrade() {
                        let clone = handle.clone().into_dyn();
                        assert!(completion.result().is_none());
                        drop(handle);
                        assert!(completion.result().is_none());
                        drop(clone);
                    }
                    Thread::yield_now();
                }
            })
            .spawn(),
        );
    }
    queue.wait_until(|| (ready.load(Ordering::Acquire) == NUM_THREADS).then_some(()));
    start.store(true, Ordering::Release);
    queue.wake_all();
    drop(owner);
    for thread in threads {
        thread.join();
    }
    completion.wait().unwrap();
    assert_eq!(memory.sync_count.load(Ordering::Acquire), 1);
    assert!(FsHandle::try_acquire(memory).is_none());
}
