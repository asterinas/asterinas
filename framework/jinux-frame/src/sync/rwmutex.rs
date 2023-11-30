use core::cell::UnsafeCell;
use core::fmt;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering::{AcqRel, Acquire, Relaxed, Release};

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
/// It is necessary for `T` to satisfy `Send` to be shared across tasks and
/// `Sync` to permit concurrent access via readers. The `Deref` method (and
/// `DerefMut` for the writer) is implemented for the RAII guards returned
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
/// use jinux_frame::sync::RwMutex;
///
/// let mutex = RwMutex::new(5)
///
/// // many read mutexs can be held at once
/// {
///     let r1 = mutex.read();
///     let r2 = mutex.read();
///     assert_eq!(*r1, 5);
///     assert_eq!(*r2, 5);
///     
///     // Upgradeable read mutex can share access to data with read mutexs
///     let r3 = mutex.upread();
///     assert_eq!(*r3, 5);
///     drop(r1);
///     drop(r2);
///     // read mutexs are dropped at this point
///
///     // An upread mutex can only be upgraded successfully after all the
///     // read mutexs are released, otherwise it will spin-wait.
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
pub struct RwMutex<T> {
    val: UnsafeCell<T>,
    /// The internal representation of the mutex state is as follows:
    /// - **Bit 63:** Writer mutex.
    /// - **Bit 62:** Upgradeable reader mutex.
    /// - **Bit 61:** Indicates if an upgradeable reader is being upgraded.
    /// - **Bits 60-0:** Reader mutex count.
    lock: AtomicUsize,
    /// Threads that fail to acquire the mutex will sleep on this waitqueue.
    queue: WaitQueue,
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

    /// Acquire a read mutex and sleep until it can be acquired.
    ///
    /// The calling thread will sleep until there are no writers or upgrading
    /// upreaders present. The implementation of `WaitQueue` guarantees the
    /// order in which other concurrent readers or writers waiting simultaneously
    /// will acquire the mutex.
    pub fn read(&self) -> RwMutexReadGuard<T> {
        self.queue.wait_until(|| self.try_read())
    }

    /// Acquire a write mutex and sleep until it can be acquired.
    ///
    /// The calling thread will sleep until there are no writers, upreaders,
    /// or readers present. The implementation of `WaitQueue` guarantees the
    /// order in which other concurrent readers or writers waiting simultaneously
    /// will acquire the mutex.
    pub fn write(&self) -> RwMutexWriteGuard<T> {
        self.queue.wait_until(|| self.try_write())
    }

    /// Acquire a upread mutex and sleep until it can be acquired.
    ///
    /// The calling thread will sleep until there are no writers or upreaders present.
    /// The implementation of `WaitQueue` guarantees the order in which other concurrent
    /// readers or writers waiting simultaneously will acquire the mutex.
    ///
    /// Upreader will not block new readers until it tries to upgrade. Upreader
    /// and reader do not differ before invoking the upgread method. However,
    /// only one upreader can exist at any time to avoid deadlock in the
    /// upgread method.
    pub fn upread(&self) -> RwMutexUpgradeableGuard<T> {
        self.queue.wait_until(|| self.try_upread())
    }

    /// Attempt to acquire a read mutex.
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

    /// Attempt to acquire a write mutex.
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

    /// Attempt to acquire a upread mutex.
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

impl<T: fmt::Debug> fmt::Debug for RwMutex<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.val, f)
    }
}

/// Because there can be more than one readers to get the T's immutable ref,
/// so T must be Sync to guarantee the sharing safety.
unsafe impl<T: Send> Send for RwMutex<T> {}
unsafe impl<T: Send + Sync> Sync for RwMutex<T> {}

impl<'a, T> !Send for RwMutexWriteGuard<'a, T> {}
unsafe impl<T: Sync> Sync for RwMutexWriteGuard<'_, T> {}

impl<'a, T> !Send for RwMutexReadGuard<'a, T> {}
unsafe impl<T: Sync> Sync for RwMutexReadGuard<'_, T> {}

impl<'a, T> !Send for RwMutexUpgradeableGuard<'a, T> {}
unsafe impl<T: Sync> Sync for RwMutexUpgradeableGuard<'_, T> {}

/// A guard that provides immutable data access.
pub struct RwMutexReadGuard<'a, T> {
    inner: &'a RwMutex<T>,
}

impl<'a, T> Deref for RwMutexReadGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.inner.val.get() }
    }
}

impl<'a, T> Drop for RwMutexReadGuard<'a, T> {
    fn drop(&mut self) {
        // When there are no readers, wake up a waiting writer.
        if self.inner.lock.fetch_sub(READER, Release) == READER {
            self.inner.queue.wake_one();
        }
    }
}

/// A guard that provides mutable data access.
pub struct RwMutexWriteGuard<'a, T> {
    inner: &'a RwMutex<T>,
}

impl<'a, T> Deref for RwMutexWriteGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.inner.val.get() }
    }
}

impl<'a, T> DerefMut for RwMutexWriteGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.inner.val.get() }
    }
}

impl<'a, T> Drop for RwMutexWriteGuard<'a, T> {
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
/// upgraded to `RwMutexWriteGuard`.
pub struct RwMutexUpgradeableGuard<'a, T> {
    inner: &'a RwMutex<T>,
}

impl<'a, T> RwMutexUpgradeableGuard<'a, T> {
    /// Upgrade this upread guard to a write guard atomically.
    ///
    /// After calling this method, subsequent readers will be blocked
    /// while previous readers remain unaffected.
    ///
    /// The calling thread will not sleep, but spin to wait for the existing
    /// reader to be released. There are two main reasons.
    /// - First, it needs to sleep in an extra waiting queue and needs extra wake-up logic and overhead.
    /// - Second, upgrading method usually requires a high response time (because the mutex is being used now).
    pub fn upgrade(mut self) -> RwMutexWriteGuard<'a, T> {
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
    pub fn try_upgrade(self) -> Result<RwMutexWriteGuard<'a, T>, Self> {
        let inner = self.inner;
        let res = self.inner.lock.compare_exchange(
            UPGRADEABLE_READER | BEING_UPGRADED,
            WRITER | UPGRADEABLE_READER,
            AcqRel,
            Relaxed,
        );
        if res.is_ok() {
            drop(self);
            Ok(RwMutexWriteGuard { inner })
        } else {
            Err(self)
        }
    }
}

impl<'a, T> Deref for RwMutexUpgradeableGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.inner.val.get() }
    }
}

impl<'a, T> Drop for RwMutexUpgradeableGuard<'a, T> {
    fn drop(&mut self) {
        let res = self.inner.lock.fetch_sub(UPGRADEABLE_READER, Release);
        if res == 0 {
            self.inner.queue.wake_all();
        }
    }
}
