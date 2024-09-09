// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::{
    cell::UnsafeCell,
    fmt,
    ops::{Deref, DerefMut},
    sync::atomic::{
        AtomicUsize,
        Ordering::{AcqRel, Acquire, Relaxed, Release},
    },
};

use super::WaitQueue;

/// A mutex that provides data access to either one writer or many readers.
///
/// # Overview
///
/// This mutex allows for multiple readers, or at most one writer to access
/// at any point in time. The writer of this mutex has exclusive access to
/// modify the underlying data, while the readers are allowed shared and
/// read-only access.
///
/// The writing and reading portions cannot be active simultaneously, when
/// one portion is in progress, the other portion will sleep. This is
/// suitable for scenarios where the mutex is expected to be held for a
/// period of time, which can avoid wasting CPU resources.
///
/// This implementation provides the upgradeable read mutex (`upread mutex`).
/// The `upread mutex` can be upgraded to write mutex atomically, useful in
/// scenarios where a decision to write is made after reading.
///
/// The type parameter `T` represents the data that this mutex is protecting.
/// It is necessary for `T` to satisfy [`Send`] to be shared across tasks and
/// [`Sync`] to permit concurrent access via readers. The [`Deref`] method (and
/// [`DerefMut`] for the writer) is implemented for the RAII guards returned
/// by the locking methods, which allows for the access to the protected data
/// while the mutex is held.
///
/// # Usage
/// The mutex can be used in scenarios where data needs to be read frequently
/// but written to occasionally.
///
/// Use `upread mutex` in scenarios where related checking is performed before
/// modification to effectively avoid deadlocks and improve efficiency.
///
/// # Safety
///
/// Avoid using `RwMutex` in an interrupt context, as it may result in sleeping
/// and never being awakened.
///
/// # Examples
///
/// ```
/// use ostd::sync::RwMutex;
///
/// let mutex = RwMutex::new(5)
///
/// // many read mutexes can be held at once
/// {
///     let r1 = mutex.read();
///     let r2 = mutex.read();
///     assert_eq!(*r1, 5);
///     assert_eq!(*r2, 5);
///     
///     // Upgradeable read mutex can share access to data with read mutexes
///     let r3 = mutex.upread();
///     assert_eq!(*r3, 5);
///     drop(r1);
///     drop(r2);
///     // read mutexes are dropped at this point
///
///     // An upread mutex can only be upgraded successfully after all the
///     // read mutexes are released, otherwise it will spin-wait.
///     let mut w1 = r3.upgrade();
///     *w1 += 1;
///     assert_eq!(*w1, 6);
/// }   // upread mutex are dropped at this point
///
/// {   
///     // Only one write mutex can be held at a time
///     let mut w2 = mutex.write();
///     *w2 += 1;
///     assert_eq!(*w2, 7);
/// }   // write mutex is dropped at this point
/// ```
pub struct RwMutex<T: ?Sized> {
    /// The internal representation of the mutex state is as follows:
    /// - **Bit 63:** Writer mutex.
    /// - **Bit 62:** Upgradeable reader mutex.
    /// - **Bit 61:** Indicates if an upgradeable reader is being upgraded.
    /// - **Bits 60-0:** Reader mutex count.
    lock: AtomicUsize,
    /// Threads that fail to acquire the mutex will sleep on this waitqueue.
    queue: WaitQueue,
    val: UnsafeCell<T>,
}

const READER: usize = 1;
const WRITER: usize = 1 << (usize::BITS - 1);
const UPGRADEABLE_READER: usize = 1 << (usize::BITS - 2);
const BEING_UPGRADED: usize = 1 << (usize::BITS - 3);
const MAX_READER: usize = 1 << (usize::BITS - 4);

impl<T> RwMutex<T> {
    /// Creates a new read-write mutex with an initial value.
    pub const fn new(val: T) -> Self {
        Self {
            val: UnsafeCell::new(val),
            lock: AtomicUsize::new(0),
            queue: WaitQueue::new(),
        }
    }
}

impl<T: ?Sized> RwMutex<T> {
    /// Acquires a read mutex and sleep until it can be acquired.
    ///
    /// The calling thread will sleep until there are no writers or upgrading
    /// upreaders present. The implementation of [`WaitQueue`] guarantees the
    /// order in which other concurrent readers or writers waiting simultaneously
    /// will acquire the mutex.
    pub fn read(&self) -> RwMutexReadGuard<T> {
        self.queue.wait_until(|| self.try_read())
    }

    /// Acquires a write mutex and sleep until it can be acquired.
    ///
    /// The calling thread will sleep until there are no writers, upreaders,
    /// or readers present. The implementation of [`WaitQueue`] guarantees the
    /// order in which other concurrent readers or writers waiting simultaneously
    /// will acquire the mutex.
    pub fn write(&self) -> RwMutexWriteGuard<T> {
        self.queue.wait_until(|| self.try_write())
    }

    /// Acquires a upread mutex and sleep until it can be acquired.
    ///
    /// The calling thread will sleep until there are no writers or upreaders present.
    /// The implementation of [`WaitQueue`] guarantees the order in which other concurrent
    /// readers or writers waiting simultaneously will acquire the mutex.
    ///
    /// Upreader will not block new readers until it tries to upgrade. Upreader
    /// and reader do not differ before invoking the upgread method. However,
    /// only one upreader can exist at any time to avoid deadlock in the
    /// upgread method.
    pub fn upread(&self) -> RwMutexUpgradeableGuard<T> {
        self.queue.wait_until(|| self.try_upread())
    }

    /// Attempts to acquire a read mutex.
    ///
    /// This function will never sleep and will return immediately.
    pub fn try_read(&self) -> Option<RwMutexReadGuard<T>> {
        let lock = self.lock.fetch_add(READER, Acquire);
        if lock & (WRITER | BEING_UPGRADED | MAX_READER) == 0 {
            Some(RwMutexReadGuard { inner: self })
        } else {
            self.lock.fetch_sub(READER, Release);
            None
        }
    }

    /// Attempts to acquire a write mutex.
    ///
    /// This function will never sleep and will return immediately.
    pub fn try_write(&self) -> Option<RwMutexWriteGuard<T>> {
        if self
            .lock
            .compare_exchange(0, WRITER, Acquire, Relaxed)
            .is_ok()
        {
            Some(RwMutexWriteGuard { inner: self })
        } else {
            None
        }
    }

    /// Attempts to acquire a upread mutex.
    ///
    /// This function will never sleep and will return immediately.
    pub fn try_upread(&self) -> Option<RwMutexUpgradeableGuard<T>> {
        let lock = self.lock.fetch_or(UPGRADEABLE_READER, Acquire) & (WRITER | UPGRADEABLE_READER);
        if lock == 0 {
            return Some(RwMutexUpgradeableGuard { inner: self });
        } else if lock == WRITER {
            self.lock.fetch_sub(UPGRADEABLE_READER, Release);
        }
        None
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for RwMutex<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.val, f)
    }
}

/// Because there can be more than one readers to get the T's immutable ref,
/// so T must be Sync to guarantee the sharing safety.
unsafe impl<T: ?Sized + Send> Send for RwMutex<T> {}
unsafe impl<T: ?Sized + Send + Sync> Sync for RwMutex<T> {}

impl<T: ?Sized, R: Deref<Target = RwMutex<T>>> !Send for RwMutexWriteGuard_<T, R> {}
unsafe impl<T: ?Sized + Sync, R: Deref<Target = RwMutex<T>> + Sync> Sync
    for RwMutexWriteGuard_<T, R>
{
}

impl<T: ?Sized, R: Deref<Target = RwMutex<T>>> !Send for RwMutexReadGuard_<T, R> {}
unsafe impl<T: ?Sized + Sync, R: Deref<Target = RwMutex<T>> + Sync> Sync
    for RwMutexReadGuard_<T, R>
{
}

impl<T: ?Sized, R: Deref<Target = RwMutex<T>>> !Send for RwMutexUpgradeableGuard_<T, R> {}
unsafe impl<T: ?Sized + Sync, R: Deref<Target = RwMutex<T>> + Sync> Sync
    for RwMutexUpgradeableGuard_<T, R>
{
}

/// A guard that provides immutable data access.
pub struct RwMutexReadGuard_<T: ?Sized, R: Deref<Target = RwMutex<T>>> {
    inner: R,
}

/// A guard that provides shared read access to the data protected by a [`RwMutex`].
pub type RwMutexReadGuard<'a, T> = RwMutexReadGuard_<T, &'a RwMutex<T>>;
/// A guard that provides shared read access to the data protected by a `Arc<RwMutex>`.
pub type ArcRwMutexReadGuard<T> = RwMutexReadGuard_<T, Arc<RwMutex<T>>>;

impl<T: ?Sized, R: Deref<Target = RwMutex<T>>> Deref for RwMutexReadGuard_<T, R> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.inner.val.get() }
    }
}

impl<T: ?Sized, R: Deref<Target = RwMutex<T>>> Drop for RwMutexReadGuard_<T, R> {
    fn drop(&mut self) {
        // When there are no readers, wake up a waiting writer.
        if self.inner.lock.fetch_sub(READER, Release) == READER {
            self.inner.queue.wake_one();
        }
    }
}

/// A guard that provides mutable data access.
#[clippy::has_significant_drop]
#[must_use]
pub struct RwMutexWriteGuard_<T: ?Sized, R: Deref<Target = RwMutex<T>>> {
    inner: R,
}

/// A guard that provides exclusive write access to the data protected by a [`RwMutex`].
pub type RwMutexWriteGuard<'a, T> = RwMutexWriteGuard_<T, &'a RwMutex<T>>;
/// A guard that provides exclusive write access to the data protected by a `Arc<RwMutex>`.
pub type ArcRwMutexWriteGuard<T> = RwMutexWriteGuard_<T, Arc<RwMutex<T>>>;

impl<T: ?Sized, R: Deref<Target = RwMutex<T>>> Deref for RwMutexWriteGuard_<T, R> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.inner.val.get() }
    }
}

impl<T: ?Sized, R: Deref<Target = RwMutex<T>> + Clone> RwMutexWriteGuard_<T, R> {
    /// Atomically downgrades a write guard to an upgradeable reader guard.
    ///
    /// This method always succeeds because the lock is exclusively held by the writer.
    pub fn downgrade(mut self) -> RwMutexUpgradeableGuard_<T, R> {
        loop {
            self = match self.try_downgrade() {
                Ok(guard) => return guard,
                Err(e) => e,
            };
        }
    }

    /// This is not exposed as a public method to prevent intermediate lock states from affecting the
    /// downgrade process.
    fn try_downgrade(self) -> Result<RwMutexUpgradeableGuard_<T, R>, Self> {
        let inner = self.inner.clone();
        let res = self
            .inner
            .lock
            .compare_exchange(WRITER, UPGRADEABLE_READER, AcqRel, Relaxed);
        if res.is_ok() {
            drop(self);
            Ok(RwMutexUpgradeableGuard_ { inner })
        } else {
            Err(self)
        }
    }
}

impl<T: ?Sized, R: Deref<Target = RwMutex<T>>> DerefMut for RwMutexWriteGuard_<T, R> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.inner.val.get() }
    }
}

impl<T: ?Sized, R: Deref<Target = RwMutex<T>>> Drop for RwMutexWriteGuard_<T, R> {
    fn drop(&mut self) {
        self.inner.lock.fetch_and(!WRITER, Release);

        // When the current writer releases, wake up all the sleeping threads.
        // All awakened threads may include readers and writers.
        // Thanks to the `wait_until` method, either all readers
        // continue to execute or one writer continues to execute.
        self.inner.queue.wake_all();
    }
}

/// A guard that provides immutable data access but can be atomically
/// upgraded to [`RwMutexWriteGuard`].
pub struct RwMutexUpgradeableGuard_<T: ?Sized, R: Deref<Target = RwMutex<T>>> {
    inner: R,
}

/// A upgradable guard that provides read access to the data protected by a [`RwMutex`].
pub type RwMutexUpgradeableGuard<'a, T> = RwMutexUpgradeableGuard_<T, &'a RwMutex<T>>;
/// A upgradable guard that provides read access to the data protected by a `Arc<RwMutex>`.
pub type ArcRwMutexUpgradeableGuard<T> = RwMutexUpgradeableGuard_<T, Arc<RwMutex<T>>>;

impl<T: ?Sized, R: Deref<Target = RwMutex<T>> + Clone> RwMutexUpgradeableGuard_<T, R> {
    /// Upgrades this upread guard to a write guard atomically.
    ///
    /// After calling this method, subsequent readers will be blocked
    /// while previous readers remain unaffected.
    ///
    /// The calling thread will not sleep, but spin to wait for the existing
    /// reader to be released. There are two main reasons.
    /// - First, it needs to sleep in an extra waiting queue and needs extra wake-up logic and overhead.
    /// - Second, upgrading method usually requires a high response time (because the mutex is being used now).
    pub fn upgrade(mut self) -> RwMutexWriteGuard_<T, R> {
        self.inner.lock.fetch_or(BEING_UPGRADED, Acquire);
        loop {
            self = match self.try_upgrade() {
                Ok(guard) => return guard,
                Err(e) => e,
            };
        }
    }

    /// Attempts to upgrade this upread guard to a write guard atomically.
    ///
    /// This function will return immediately.
    pub fn try_upgrade(self) -> Result<RwMutexWriteGuard_<T, R>, Self> {
        let res = self.inner.lock.compare_exchange(
            UPGRADEABLE_READER | BEING_UPGRADED,
            WRITER | UPGRADEABLE_READER,
            AcqRel,
            Relaxed,
        );
        if res.is_ok() {
            let inner = self.inner.clone();
            drop(self);
            Ok(RwMutexWriteGuard_ { inner })
        } else {
            Err(self)
        }
    }
}

impl<T: ?Sized, R: Deref<Target = RwMutex<T>>> Deref for RwMutexUpgradeableGuard_<T, R> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.inner.val.get() }
    }
}

impl<T: ?Sized, R: Deref<Target = RwMutex<T>>> Drop for RwMutexUpgradeableGuard_<T, R> {
    fn drop(&mut self) {
        let res = self.inner.lock.fetch_sub(UPGRADEABLE_READER, Release);
        if res == UPGRADEABLE_READER {
            self.inner.queue.wake_all();
        }
    }
}
