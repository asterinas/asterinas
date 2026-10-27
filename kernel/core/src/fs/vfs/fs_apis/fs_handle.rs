// SPDX-License-Identifier: MPL-2.0

//! Active filesystem references, independent of allocation lifetime.
//!
//! A filesystem instance is referenced in two different ways:
//!
//! - An [`FsHandle`] is an _active_ reference.
//!   It keeps the filesystem in use and thus defers its final shutdown.
//!   When the last `FsHandle` is released,
//!   the filesystem is permanently retired and [`FileSystem::shutdown`] runs exactly once.
//! - An `Arc<dyn FileSystem>` (or `Arc<F>`) only keeps the filesystem _allocated_.
//!   It does not prevent shutdown,
//!   and it remains usable during and after shutdown.
//!   A `Weak<F>` keeps neither; it upgrades only while some `Arc` remains.
//!
//! # Choosing between the two
//!
//! An object holds an `FsHandle` only if it may keep the filesystem in use
//! beyond the lifetime of every other active reference.
//! Such objects are the roots of filesystem usage:
//! [`Mount`]s, newly created filesystems waiting to be mounted (see [`FsAndRoot`]),
//! filesystems stacked on others (e.g., overlayfs layers),
//! kernel-internal singletons that are never unmounted,
//! and deferred work that may run after its owner is gone
//! (e.g., the page cache flush that virtiofs defers past closing a file).
//!
//! Any other object should hold an `Arc` or a `Weak`.
//! This includes objects whose lifetime is covered by an active owner
//! (e.g., an opened file covered by the mount of its path),
//! filesystem-internal objects such as inodes,
//! and temporary references used for a single operation.
//! In particular, [`Inode::fs`] returns an `Arc<dyn FileSystem>`:
//! retaining an inode does not keep its filesystem in use.
//! Filesystem-internal code may acquire an `FsHandle` only while an active owner
//! is known to exist (e.g., when a file is opened through a path),
//! which amounts to cloning the owner's handle.
//! Such acquisition would fail during shutdown,
//! which runs after the last handle is released.
//!
//! [`Mount`]: crate::fs::vfs::path::Mount
//! [`FsAndRoot`]: super::registry::FsAndRoot
//! [`FsCache`]: super::registry::FsCache
//! [`Inode::fs`]: super::inode::Inode::fs
//!
//! # Lifecycle
//!
//! Each filesystem embeds an [`FsLifecycle`].
//! A filesystem starts as inactive,
//! becomes active when its first handle is created by [`FsHandle::new`],
//! and is retired when its last handle is released.
//! A retired filesystem can never be activated again;
//! [`FsCache`] instead waits for its shutdown to complete before creating a replacement.

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
///
/// See the [module-level documentation](self) for when to hold an `FsHandle`
/// instead of an `Arc<dyn FileSystem>`.
pub(crate) struct FsHandle<F: FileSystem + ?Sized = dyn FileSystem> {
    // Taken during type erasure or final release without duplicating the Arc.
    fs: Option<Arc<F>>,
}

impl<F: FileSystem> FsHandle<F> {
    /// Erases the filesystem type without changing either reference count.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(in crate::fs) fn into_dyn(mut self) -> FsHandle {
        FsHandle {
            fs: Some(self.fs.take().unwrap()),
        }
    }
}

impl<F: FileSystem + ?Sized> FsHandle<F> {
    /// Activates a fully initialized filesystem and returns its first handle.
    ///
    /// Each filesystem instance can be activated only once.
    /// Further handles are obtained by cloning the returned handle.
    ///
    /// # Panics
    ///
    /// Panics if `fs` has already been activated.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(in crate::fs) fn new(fs: Arc<F>) -> Self {
        let activated = fs.lifecycle().num_handles.compare_exchange(
            FsLifecycle::INITIALIZING,
            1,
            Ordering::Release,
            Ordering::Relaxed,
        );
        assert!(activated.is_ok(), "filesystem was already activated");
        Self { fs: Some(fs) }
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

    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(super) fn downgrade(&self) -> WeakFsHandle<F> {
        WeakFsHandle(Arc::downgrade(self.as_arc()))
    }

    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(super) fn shutdown_completion(&self) -> &Arc<FsShutdown> {
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
pub(super) struct WeakFsHandle<F: FileSystem + ?Sized = dyn FileSystem>(Weak<F>);

impl<F: FileSystem + ?Sized> WeakFsHandle<F> {
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(super) fn upgrade(&self) -> Option<FsHandle<F>> {
        FsHandle::try_acquire(self.0.upgrade()?)
    }
}

impl<F: FileSystem + ?Sized> Clone for WeakFsHandle<F> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// Completion of final cleanup, independent of filesystem allocation lifetime.
pub(super) struct FsShutdown {
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

    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(super) fn wait(&self) -> Result<()> {
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
