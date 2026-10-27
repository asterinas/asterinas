// SPDX-License-Identifier: MPL-2.0

//! Active filesystem references, independent of allocation lifetime.
//!
//! Each filesystem embeds `FsLifecycle`. Internal `Arc<F>` and `Weak<F>`
//! references remain usable during shutdown without counting as active users.

use core::{
    ops::Deref,
    sync::atomic::{self, AtomicI32, AtomicUsize, Ordering},
};

use ostd::sync::WaitQueue;

use super::file_system::FileSystem;
use crate::{
    prelude::*,
    thread::work_queue::{self, WorkPriority},
};

#[cfg(ktest)]
mod tests;

/// Lifecycle state embedded in a filesystem instance.
pub(crate) struct FsLifecycle {
    // INITIALIZING -> positive handle count -> 0 (permanently retired).
    num_handles: AtomicUsize,
    shutdown: Arc<FsShutdown>,
}

impl FsLifecycle {
    const INITIALIZING: usize = usize::MAX;

    pub(in crate::fs) fn new() -> Self {
        Self {
            num_handles: AtomicUsize::new(Self::INITIALIZING),
            shutdown: Arc::new(FsShutdown::new()),
        }
    }

    fn try_acquire(&self) -> bool {
        self.num_handles
            .try_update(Ordering::Acquire, Ordering::Relaxed, |count| {
                if count == 0 || count == Self::INITIALIZING {
                    return None;
                }
                assert!(count < isize::MAX as usize, "too many filesystem handles");
                Some(count + 1)
            })
            .is_ok()
    }
}

impl Debug for FsLifecycle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FsLifecycle")
            .field("num_handles", &self.num_handles.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

/// An active reference that prevents final filesystem shutdown.
pub(crate) struct FsHandle<F: FileSystem + ?Sized = dyn FileSystem> {
    // Taken during type erasure or final release without duplicating the Arc.
    fs: Option<Arc<F>>,
}

impl<F: FileSystem> FsHandle<F> {
    pub(in crate::fs) fn new(fs: F) -> Self {
        Self::activate(Arc::new(fs)).unwrap()
    }

    pub(in crate::fs) fn new_cyclic(create_fn: impl FnOnce(&Weak<F>) -> F) -> Self {
        Self::activate(Arc::new_cyclic(create_fn)).unwrap()
    }

    /// Erases the filesystem type without changing either reference count.
    pub(in crate::fs) fn into_dyn(mut self) -> FsHandle {
        FsHandle {
            fs: Some(self.fs.take().unwrap()),
        }
    }
}

impl<F: FileSystem + ?Sized> FsHandle<F> {
    /// Publishes a fully initialized filesystem exactly once.
    pub(in crate::fs) fn activate(fs: Arc<F>) -> Result<Self> {
        fs.lifecycle()
            .num_handles
            .compare_exchange(
                FsLifecycle::INITIALIZING,
                1,
                Ordering::Release,
                Ordering::Relaxed,
            )
            .map_err(|_| Error::with_message(Errno::EBUSY, "filesystem was already activated"))?;
        Ok(Self { fs: Some(fs) })
    }

    /// Acquires an active reference, failing before activation or after retirement.
    pub(in crate::fs) fn try_acquire(fs: Arc<F>) -> Option<Self> {
        if !fs.lifecycle().try_acquire() {
            return None;
        }
        Some(Self { fs: Some(fs) })
    }

    /// Borrows the memory reference for filesystem-internal use.
    pub(in crate::fs) fn as_arc(&self) -> &Arc<F> {
        self.fs.as_ref().unwrap()
    }

    pub(in crate::fs) fn downgrade(&self) -> WeakFsHandle<F> {
        WeakFsHandle(Arc::downgrade(self.as_arc()))
    }

    pub(in crate::fs) fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(self.as_arc(), other.as_arc())
    }

    pub(in crate::fs) fn shutdown_completion(&self) -> &Arc<FsShutdown> {
        &self.lifecycle().shutdown
    }

    fn queue_shutdown(fs: Arc<F>) {
        let shutdown = fs.lifecycle().shutdown.clone();
        let pending: SpinLock<_> = SpinLock::new(Some(fs));
        work_queue::submit_work_func(
            move || {
                // The submitter may release the last work-item reference under a
                // spinlock, so the worker takes ownership before performing cleanup.
                let fs = pending.lock().take().unwrap();
                let result = fs.shutdown();
                if let Err(err) = result {
                    error!(
                        "failed to sync filesystem {} during final cleanup: {:?}",
                        fs.name(),
                        err
                    );
                }
                drop(fs);
                shutdown.complete(result);
            },
            WorkPriority::Normal,
        );
    }
}

impl<F: FileSystem + ?Sized> Clone for FsHandle<F> {
    fn clone(&self) -> Self {
        Self::try_acquire(self.as_arc().clone()).unwrap()
    }
}

impl<F: FileSystem + ?Sized> Deref for FsHandle<F> {
    type Target = F;

    fn deref(&self) -> &Self::Target {
        self.as_arc()
    }
}

impl<F: FileSystem + ?Sized> AsRef<F> for FsHandle<F> {
    fn as_ref(&self) -> &F {
        self
    }
}

impl<F: FileSystem + ?Sized> Debug for FsHandle<F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FsHandle")
            .field("name", &self.name())
            .finish_non_exhaustive()
    }
}

impl<F: FileSystem + ?Sized> Drop for FsHandle<F> {
    fn drop(&mut self) {
        let Some(fs) = self.fs.take() else {
            return;
        };
        let previous = fs.lifecycle().num_handles.fetch_sub(1, Ordering::Release);
        debug_assert!(previous > 0 && previous != FsLifecycle::INITIALIZING);
        if previous == 1 {
            atomic::fence(Ordering::Acquire);
            Self::queue_shutdown(fs);
        }
    }
}

/// A weak reference that only upgrades while the filesystem remains active.
pub(crate) struct WeakFsHandle<F: FileSystem + ?Sized = dyn FileSystem>(Weak<F>);

impl<F: FileSystem + ?Sized> WeakFsHandle<F> {
    pub(in crate::fs) fn from_weak(fs: Weak<F>) -> Self {
        Self(fs)
    }

    pub(in crate::fs) fn upgrade(&self) -> Option<FsHandle<F>> {
        FsHandle::try_acquire(self.0.upgrade()?)
    }
}

impl<F: FileSystem> WeakFsHandle<F> {
    pub(in crate::fs) fn into_dyn(self) -> WeakFsHandle {
        WeakFsHandle(self.0)
    }
}

impl<F: FileSystem + ?Sized> Clone for WeakFsHandle<F> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// Completion of final cleanup, independent of filesystem allocation lifetime.
pub(in crate::fs) struct FsShutdown {
    result: AtomicI32,
    wait_queue: WaitQueue,
}

impl FsShutdown {
    const PENDING: i32 = 0;
    const SUCCESS: i32 = 1;

    fn new() -> Self {
        Self {
            result: AtomicI32::new(Self::PENDING),
            wait_queue: WaitQueue::new(),
        }
    }

    pub(super) fn result(&self) -> Option<Result<()>> {
        match self.result.load(Ordering::Acquire) {
            Self::PENDING => None,
            Self::SUCCESS => Some(Ok(())),
            result => Some(Err(Errno::try_from(-result).unwrap().into())),
        }
    }

    pub(in crate::fs) fn wait(&self) -> Result<()> {
        self.wait_queue.wait_until(|| self.result())
    }

    fn complete(&self, result: Result<()>) {
        debug_assert_eq!(self.result.load(Ordering::Relaxed), Self::PENDING);
        self.result.store(
            match result {
                Ok(()) => Self::SUCCESS,
                Err(err) => -(err.error() as i32),
            },
            Ordering::Release,
        );
        self.wait_queue.wake_all();
    }
}
