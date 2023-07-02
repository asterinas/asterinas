use core::cell::UnsafeCell;
use core::fmt;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering::{Acquire, Relaxed, Release};

use crate::trap::disable_local;
use crate::trap::DisabledLocalIrqGuard;

/// A read write lock, waiting by spinning.
/// Now, the lock's layout is simply like:
/// ```
/// bit:       63     |        62 ~ 0
/// use:  writer lock | reader lock & numbers
/// ```
pub struct RwLock<T> {
    val: UnsafeCell<T>,
    lock: AtomicUsize,
}

const READER: usize = 1;
const WRITER: usize = 1 << (usize::BITS - 1);
const MAX_READER: usize = WRITER >> 1;

impl<T> RwLock<T> {
    /// Creates a new read/write lock.
    pub const fn new(val: T) -> Self {
        Self {
            val: UnsafeCell::new(val),
            lock: AtomicUsize::new(0),
        }
    }

    /// Acquire a read lock with disabling the local IRQs. This is the most secure
    /// locking method.
    ///
    /// This method runs in a busy loop until the lock can be acquired (when there are
    /// no writers).
    /// After acquiring the spin lock, all interrupts are disabled.
    pub fn read_irq_disabled(&self) -> RwLockReadIrqDisabledGuard<T> {
        loop {
            if let Some(readguard) = self.try_read_irq_disabled() {
                return readguard;
            } else {
                core::hint::spin_loop();
            }
        }
    }

    /// Acquire a write lock with disabling local IRQs. This is the most secure
    /// locking method.
    ///
    /// This method runs in a busy loop until the lock can be acquired (when there are
    /// no writers and readers).
    /// After acquiring the spin lock, all interrupts are disabled.
    pub fn write_irq_disabled(&self) -> RwLockWriteIrqDisabledGuard<T> {
        loop {
            if let Some(writeguard) = self.try_write_irq_disabled() {
                return writeguard;
            } else {
                core::hint::spin_loop();
            }
        }
    }

    /// Try acquire a read lock with disabling local IRQs.
    pub fn try_read_irq_disabled(&self) -> Option<RwLockReadIrqDisabledGuard<T>> {
        // FIXME: add disable_preemption
        let irq_guard = disable_local();
        let lock = self.lock.fetch_add(READER, Acquire);
        if lock & (WRITER | MAX_READER) == 0 {
            Some(RwLockReadIrqDisabledGuard {
                inner: &self,
                irq_guard,
            })
        } else {
            self.lock.fetch_sub(READER, Release);
            None
        }
    }

    /// Try acquire a write lock with disabling local IRQs.
    pub fn try_write_irq_disabled(&self) -> Option<RwLockWriteIrqDisabledGuard<T>> {
        // FIXME: add disable_preemption
        let irq_guard = disable_local();
        if self
            .lock
            .compare_exchange(0, WRITER, Acquire, Relaxed)
            .is_ok()
        {
            Some(RwLockWriteIrqDisabledGuard {
                inner: &self,
                irq_guard,
            })
        } else {
            None
        }
    }

    /// Acquire a read lock without disabling local IRQs.
    ///
    /// Prefer using this method over the `read_irq_disabled` method
    /// when IRQ handlers are allowed to get executed while
    /// holding this lock. For example, if a lock is never used
    /// in the interrupt context, then it is ok to use this method
    /// in the process context.
    pub fn read(&self) -> RwLockReadGuard<T> {
        loop {
            if let Some(readguard) = self.try_read() {
                return readguard;
            } else {
                core::hint::spin_loop();
            }
        }
    }

    /// Acquire a write lock without disabling local IRQs.
    pub fn write(&self) -> RwLockWriteGuard<T> {
        loop {
            if let Some(writeguard) = self.try_write() {
                return writeguard;
            } else {
                core::hint::spin_loop();
            }
        }
    }

    /// Try acquire a read lock without disabling the local IRQs.
    pub fn try_read(&self) -> Option<RwLockReadGuard<T>> {
        // FIXME: add disable_preemption
        let lock = self.lock.fetch_add(READER, Acquire);
        if lock & (WRITER | MAX_READER) == 0 {
            Some(RwLockReadGuard { inner: &self })
        } else {
            self.lock.fetch_sub(READER, Release);
            None
        }
    }

    /// Try acquire a write lock without disabling the local IRQs.
    pub fn try_write(&self) -> Option<RwLockWriteGuard<T>> {
        // FIXME: add disable_preemption
        if self
            .lock
            .compare_exchange(0, WRITER, Acquire, Relaxed)
            .is_ok()
        {
            Some(RwLockWriteGuard { inner: &self })
        } else {
            None
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for RwLock<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.val, f)
    }
}

/// Because there can be more than one readers to get the T's immutable ref,
/// so T must be Sync to guarantee the sharing safety.
unsafe impl<T: Send> Send for RwLock<T> {}
unsafe impl<T: Send + Sync> Sync for RwLock<T> {}

impl<'a, T> !Send for RwLockWriteIrqDisabledGuard<'a, T> {}
unsafe impl<T: Sync> Sync for RwLockWriteIrqDisabledGuard<'_, T> {}

impl<'a, T> !Send for RwLockReadIrqDisabledGuard<'a, T> {}
unsafe impl<T: Sync> Sync for RwLockReadIrqDisabledGuard<'_, T> {}

/// The guard of a read lock that disables the local IRQs.
pub struct RwLockReadIrqDisabledGuard<'a, T> {
    inner: &'a RwLock<T>,
    irq_guard: DisabledLocalIrqGuard,
}

/// Upgrade a read lock that disables the local IRQs to a write lock.
///
/// This method first release the old read lock and then aquire a new write lock.
/// So it may not return the guard immidiately
/// due to other readers or another writer.
impl<'a, T> RwLockReadIrqDisabledGuard<'a, T> {
    pub fn upgrade(mut self) -> RwLockWriteIrqDisabledGuard<'a, T> {
        let inner = self.inner;
        let irq_guard = self.irq_guard.transfer_to();
        drop(self);
        while inner
            .lock
            .compare_exchange(0, WRITER, Acquire, Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        RwLockWriteIrqDisabledGuard { inner, irq_guard }
    }
}

impl<'a, T> Deref for RwLockReadIrqDisabledGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.inner.val.get() }
    }
}

impl<'a, T> Drop for RwLockReadIrqDisabledGuard<'a, T> {
    fn drop(&mut self) {
        self.inner.lock.fetch_sub(READER, Release);
    }
}

/// The guard of a write lock that disables the local IRQs.
pub struct RwLockWriteIrqDisabledGuard<'a, T> {
    inner: &'a RwLock<T>,
    irq_guard: DisabledLocalIrqGuard,
}

/// Downgrade a write lock that disables the local IRQs to a read lock.
///
/// This method can return the read guard immidiately
/// due to there must be no other users.
impl<'a, T> RwLockWriteIrqDisabledGuard<'a, T> {
    pub fn downgrade(mut self) -> RwLockReadIrqDisabledGuard<'a, T> {
        self.inner.lock.fetch_add(READER, Acquire);
        let inner = self.inner;
        let irq_guard = self.irq_guard.transfer_to();
        drop(self);
        let irq_guard = disable_local();
        RwLockReadIrqDisabledGuard { inner, irq_guard }
    }
}

impl<'a, T> Deref for RwLockWriteIrqDisabledGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.inner.val.get() }
    }
}

impl<'a, T> DerefMut for RwLockWriteIrqDisabledGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.inner.val.get() }
    }
}

impl<'a, T> Drop for RwLockWriteIrqDisabledGuard<'a, T> {
    fn drop(&mut self) {
        self.inner.lock.fetch_and(!(WRITER), Release);
    }
}

impl<'a, T> !Send for RwLockWriteGuard<'a, T> {}
unsafe impl<T: Sync> Sync for RwLockWriteGuard<'_, T> {}

impl<'a, T> !Send for RwLockReadGuard<'a, T> {}
unsafe impl<T: Sync> Sync for RwLockReadGuard<'_, T> {}

/// The guard of the read lock.
pub struct RwLockReadGuard<'a, T> {
    inner: &'a RwLock<T>,
}

/// Upgrade a read lock to a write lock.
///
/// This method first release the old read lock and then aquire a new write lock.
/// So it may not return the write guard immidiately
/// due to other readers or another writer.
impl<'a, T> RwLockReadGuard<'a, T> {
    pub fn upgrade(self) -> RwLockWriteGuard<'a, T> {
        let inner = self.inner;
        drop(self);
        while inner
            .lock
            .compare_exchange(0, WRITER, Acquire, Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        RwLockWriteGuard { inner }
    }
}

impl<'a, T> Deref for RwLockReadGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.inner.val.get() }
    }
}

impl<'a, T> Drop for RwLockReadGuard<'a, T> {
    fn drop(&mut self) {
        self.inner.lock.fetch_sub(READER, Release);
    }
}

pub struct RwLockWriteGuard<'a, T> {
    inner: &'a RwLock<T>,
}

/// Downgrade a write lock to a read lock.
///
/// This method can return the read guard immidiately
/// due to there are no other users.
impl<'a, T> RwLockWriteGuard<'a, T> {
    pub fn downgrade(self) -> RwLockReadGuard<'a, T> {
        self.inner.lock.fetch_add(READER, Acquire);
        let inner = self.inner;
        drop(self);
        RwLockReadGuard { inner }
    }
}

impl<'a, T> Deref for RwLockWriteGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.inner.val.get() }
    }
}

impl<'a, T> DerefMut for RwLockWriteGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.inner.val.get() }
    }
}

impl<'a, T> Drop for RwLockWriteGuard<'a, T> {
    fn drop(&mut self) {
        self.inner.lock.fetch_and(!(WRITER), Release);
    }
}
