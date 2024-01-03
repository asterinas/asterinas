// SPDX-License-Identifier: MPL-2.0

use core::cell::UnsafeCell;
use core::fmt;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering::{AcqRel, Acquire, Relaxed, Release};

use crate::task::{disable_preempt, DisablePreemptGuard};
use crate::trap::disable_local;
use crate::trap::DisabledLocalIrqGuard;

/// Spin-based Read-write Lock
///
/// # Overview
///
/// This lock allows for multiple readers, or at most one writer to access
/// at any point in time. The writer of this lock has exclusive access to
/// modify the underlying data, while the readers are allowed shared and
/// read-only access.
///
/// The writing and reading portions cannot be active simultaneously, when
/// one portion is in progress, the other portion will spin-wait. This is
/// suitable for scenarios where the lock is expected to be held for short
/// periods of time, and the overhead of context switching is higher than
/// the cost of spinning.
///
/// The lock provides methods to safely acquire locks with interrupts
/// disabled, preventing deadlocks in scenarios where locks are used within
/// interrupt handlers.
///
/// In addition to traditional read and write locks, this implementation
/// provides the upgradeable read lock (`upread lock`). The `upread lock`
/// can be upgraded to write locks atomically, useful in scenarios
/// where a decision to write is made after reading.
///
/// The type parameter `T` represents the data that this lock is protecting.
/// It is necessary for `T` to satisfy `Send` to be shared across tasks and
/// `Sync` to permit concurrent access via readers. The `Deref` method (and
/// `DerefMut` for the writer) is implemented for the RAII guards returned
/// by the locking methods, which allows for the access to the protected data
/// while the lock is held.
///
/// # Usage
/// The lock can be used in scenarios where data needs to be read frequently
/// but written to occasionally.
///
/// Use `upread lock` in scenarios where related checking is performed before
/// modification to effectively avoid deadlocks and improve efficiency.
///
/// This lock should not be used in scenarios where lock-holding times are
/// long as it can lead to CPU resource wastage due to spinning.
///
/// # Safety
///
/// Use interrupt-disabled version methods when dealing with interrupt-related read-write locks,
///  as nested interrupts may lead to a deadlock if not properly handled.
///
/// # Examples
///
/// ```
/// use aster_frame::sync::RwLock;
///
/// let lock = RwLock::new(5)
///
/// // many read locks can be held at once
/// {
///     let r1 = lock.read();
///     let r2 = lock.read();
///     assert_eq!(*r1, 5);
///     assert_eq!(*r2, 5);
///     
///     // Upgradeable read lock can share access to data with read locks
///     let r3 = lock.upread();
///     assert_eq!(*r3, 5);
///     drop(r1);
///     drop(r2);
///     // read locks are dropped at this point
///
///     // An upread lock can only be upgraded successfully after all the
///     // read locks are released, otherwise it will spin-wait.
///     let mut w1 = r3.upgrade();
///     *w1 += 1;
///     assert_eq!(*w1, 6);
/// }   // upread lock are dropped at this point
///
/// {   
///     // Only one write lock can be held at a time
///     let mut w2 = lock.write();
///     *w2 += 1;
///     assert_eq!(*w2, 7);
/// }   // write lock is dropped at this point
/// ```
pub struct RwLock<T> {
    val: UnsafeCell<T>,
    /// The internal representation of the lock state is as follows:
    /// - **Bit 63:** Writer lock.
    /// - **Bit 62:** Upgradeable reader lock.
    /// - **Bit 61:** Indicates if an upgradeable reader is being upgraded.
    /// - **Bits 60-0:** Reader lock count.
    lock: AtomicUsize,
}

const READER: usize = 1;
const WRITER: usize = 1 << (usize::BITS - 1);
const UPGRADEABLE_READER: usize = 1 << (usize::BITS - 2);
const BEING_UPGRADED: usize = 1 << (usize::BITS - 3);
const MAX_READER: usize = 1 << (usize::BITS - 4);

impl<T> RwLock<T> {
    /// Creates a new spin-based read-write lock with an initial value.
    pub const fn new(val: T) -> Self {
        Self {
            val: UnsafeCell::new(val),
            lock: AtomicUsize::new(0),
        }
    }

    /// Acquire a read lock while disabling the local IRQs and spin-wait
    /// until it can be acquired.
    ///
    /// The calling thread will spin-wait until there are no writers or
    /// upgrading upreaders present. There is no guarantee for the order
    /// in which other readers or writers waiting simultaneously will
    /// obtain the lock. Once this lock is acquired, the calling thread
    /// will not be interrupted.
    pub fn read_irq_disabled(&self) -> RwLockReadGuard<T> {
        loop {
            if let Some(readguard) = self.try_read_irq_disabled() {
                return readguard;
            } else {
                core::hint::spin_loop();
            }
        }
    }

    /// Acquire a write lock while disabling the local IRQs and spin-wait
    /// until it can be acquired.
    ///
    /// The calling thread will spin-wait until there are no other writers,
    /// , upreaders or readers present. There is no guarantee for the order
    /// in which other readers or writers waiting simultaneously will
    /// obtain the lock. Once this lock is acquired, the calling thread
    /// will not be interrupted.
    pub fn write_irq_disabled(&self) -> RwLockWriteGuard<T> {
        loop {
            if let Some(writeguard) = self.try_write_irq_disabled() {
                return writeguard;
            } else {
                core::hint::spin_loop();
            }
        }
    }

    /// Acquire an upgradeable reader (upreader) while disabling local IRQs
    /// and spin-wait until it can be acquired.
    ///
    /// The calling thread will spin-wait until there are no other writers,
    /// or upreaders. There is no guarantee for the order in which other
    /// readers or writers waiting simultaneously will obtain the lock. Once
    /// this lock is acquired, the calling thread will not be interrupted.
    ///
    /// Upreader will not block new readers until it tries to upgrade. Upreader
    /// and reader do not differ before invoking the upgread method. However,
    /// only one upreader can exist at any time to avoid deadlock in the
    /// upgread method.
    pub fn upread_irq_disabled(&self) -> RwLockUpgradeableGuard<T> {
        loop {
            if let Some(guard) = self.try_upread_irq_disabled() {
                return guard;
            } else {
                core::hint::spin_loop();
            }
        }
    }

    /// Attempt to acquire a read lock while disabling local IRQs.
    ///
    /// This function will never spin-wait and will return immediately. When
    /// multiple readers or writers attempt to acquire the lock, this method
    /// does not guarantee any order. Interrupts will automatically be restored
    /// when acquiring fails.
    pub fn try_read_irq_disabled(&self) -> Option<RwLockReadGuard<T>> {
        let irq_guard = disable_local();
        let lock = self.lock.fetch_add(READER, Acquire);
        if lock & (WRITER | MAX_READER | BEING_UPGRADED) == 0 {
            Some(RwLockReadGuard {
                inner: self,
                inner_guard: InnerGuard::IrqGuard(irq_guard),
            })
        } else {
            self.lock.fetch_sub(READER, Release);
            None
        }
    }

    /// Attempt to acquire a write lock while disabling local IRQs.
    ///
    /// This function will never spin-wait and will return immediately. When
    /// multiple readers or writers attempt to acquire the lock, this method
    /// does not guarantee any order. Interrupts will automatically be restored
    /// when acquiring fails.
    pub fn try_write_irq_disabled(&self) -> Option<RwLockWriteGuard<T>> {
        let irq_guard = disable_local();
        if self
            .lock
            .compare_exchange(0, WRITER, Acquire, Relaxed)
            .is_ok()
        {
            Some(RwLockWriteGuard {
                inner: self,
                inner_guard: InnerGuard::IrqGuard(irq_guard),
            })
        } else {
            None
        }
    }

    /// Attempt to acquire a upread lock while disabling local IRQs.
    ///
    /// This function will never spin-wait and will return immediately. When
    /// multiple readers or writers attempt to acquire the lock, this method
    /// does not guarantee any order. Interrupts will automatically be restored
    /// when acquiring fails.
    pub fn try_upread_irq_disabled(&self) -> Option<RwLockUpgradeableGuard<T>> {
        let irq_guard = disable_local();
        let lock = self.lock.fetch_or(UPGRADEABLE_READER, Acquire) & (WRITER | UPGRADEABLE_READER);
        if lock == 0 {
            return Some(RwLockUpgradeableGuard {
                inner: self,
                inner_guard: InnerGuard::IrqGuard(irq_guard),
            });
        } else if lock == WRITER {
            self.lock.fetch_sub(UPGRADEABLE_READER, Release);
        }
        None
    }

    /// Acquire a read lock and spin-wait until it can be acquired.
    ///
    /// The calling thread will spin-wait until there are no writers or
    /// upgrading upreaders present. There is no guarantee for the order
    /// in which other readers or writers waiting simultaneously will
    /// obtain the lock.
    ///
    /// This method does not disable interrupts, so any locks related to
    /// interrupt context should avoid using this method, and use `read_irq_disabled`
    /// instead. When IRQ handlers are allowed to be executed while holding
    /// this lock, it is preferable to use this method over the `read_irq_disabled`
    /// method as it has a higher efficiency.
    pub fn read(&self) -> RwLockReadGuard<T> {
        loop {
            if let Some(readguard) = self.try_read() {
                return readguard;
            } else {
                core::hint::spin_loop();
            }
        }
    }

    /// Acquire a write lock and spin-wait until it can be acquired.
    ///
    /// The calling thread will spin-wait until there are no other writers,
    /// , upreaders or readers present. There is no guarantee for the order
    /// in which other readers or writers waiting simultaneously will
    /// obtain the lock.
    ///
    /// This method does not disable interrupts, so any locks related to
    /// interrupt context should avoid using this method, and use `write_irq_disabled`
    /// instead. When IRQ handlers are allowed to be executed while holding
    /// this lock, it is preferable to use this method over the `write_irq_disabled`
    /// method as it has a higher efficiency.
    pub fn write(&self) -> RwLockWriteGuard<T> {
        loop {
            if let Some(writeguard) = self.try_write() {
                return writeguard;
            } else {
                core::hint::spin_loop();
            }
        }
    }

    /// Acquire an upreader and spin-wait until it can be acquired.
    ///
    /// The calling thread will spin-wait until there are no other writers,
    /// or upreaders. There is no guarantee for the order in which other
    /// readers or writers waiting simultaneously will obtain the lock.
    ///
    /// Upreader will not block new readers until it tries to upgrade. Upreader
    /// and reader do not differ before invoking the upgread method. However,
    /// only one upreader can exist at any time to avoid deadlock in the
    /// upgread method.
    ///
    /// This method does not disable interrupts, so any locks related to
    /// interrupt context should avoid using this method, and use `upread_irq_disabled`
    /// instead. When IRQ handlers are allowed to be executed while holding
    /// this lock, it is preferable to use this method over the `upread_irq_disabled`
    /// method as it has a higher efficiency.
    pub fn upread(&self) -> RwLockUpgradeableGuard<T> {
        loop {
            if let Some(guard) = self.try_upread() {
                return guard;
            } else {
                core::hint::spin_loop();
            }
        }
    }

    /// Attempt to acquire a read lock.
    ///
    /// This function will never spin-wait and will return immediately.
    ///
    /// This method does not disable interrupts, so any locks related to
    /// interrupt context should avoid using this method, and use
    /// `try_read_irq_disabled` instead. When IRQ handlers are allowed to
    /// be executed while holding this lock, it is preferable to use this
    /// method over the `try_read_irq_disabled` method as it has a higher
    /// efficiency.
    pub fn try_read(&self) -> Option<RwLockReadGuard<T>> {
        let guard = disable_preempt();
        let lock = self.lock.fetch_add(READER, Acquire);
        if lock & (WRITER | MAX_READER | BEING_UPGRADED) == 0 {
            Some(RwLockReadGuard {
                inner: self,
                inner_guard: InnerGuard::PreemptGuard(guard),
            })
        } else {
            self.lock.fetch_sub(READER, Release);
            None
        }
    }

    /// Attempt to acquire a write lock.
    ///
    /// This function will never spin-wait and will return immediately.
    ///
    /// This method does not disable interrupts, so any locks related to
    /// interrupt context should avoid using this method, and use
    /// `try_write_irq_disabled` instead. When IRQ handlers are allowed to
    /// be executed while holding this lock, it is preferable to use this
    /// method over the `try_write_irq_disabled` method as it has a higher
    /// efficiency.
    pub fn try_write(&self) -> Option<RwLockWriteGuard<T>> {
        let guard = disable_preempt();
        if self
            .lock
            .compare_exchange(0, WRITER, Acquire, Relaxed)
            .is_ok()
        {
            Some(RwLockWriteGuard {
                inner: self,
                inner_guard: InnerGuard::PreemptGuard(guard),
            })
        } else {
            None
        }
    }

    /// Attempt to acquire an upread lock.
    ///
    /// This function will never spin-wait and will return immediately.
    ///
    /// This method does not disable interrupts, so any locks related to
    /// interrupt context should avoid using this method, and use
    /// `try_upread_irq_disabled` instead. When IRQ handlers are allowed to
    /// be executed while holding this lock, it is preferable to use this
    /// method over the `try_upread_irq_disabled` method as it has a higher
    /// efficiency.
    pub fn try_upread(&self) -> Option<RwLockUpgradeableGuard<T>> {
        let guard = disable_preempt();
        let lock = self.lock.fetch_or(UPGRADEABLE_READER, Acquire) & (WRITER | UPGRADEABLE_READER);
        if lock == 0 {
            return Some(RwLockUpgradeableGuard {
                inner: self,
                inner_guard: InnerGuard::PreemptGuard(guard),
            });
        } else if lock == WRITER {
            self.lock.fetch_sub(UPGRADEABLE_READER, Release);
        }
        None
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

impl<'a, T> !Send for RwLockWriteGuard<'a, T> {}
unsafe impl<T: Sync> Sync for RwLockWriteGuard<'_, T> {}

impl<'a, T> !Send for RwLockReadGuard<'a, T> {}
unsafe impl<T: Sync> Sync for RwLockReadGuard<'_, T> {}

impl<'a, T> !Send for RwLockUpgradeableGuard<'a, T> {}
unsafe impl<T: Sync> Sync for RwLockUpgradeableGuard<'_, T> {}

enum InnerGuard {
    IrqGuard(DisabledLocalIrqGuard),
    PreemptGuard(DisablePreemptGuard),
}

/// A guard that provides immutable data access.
pub struct RwLockReadGuard<'a, T> {
    inner: &'a RwLock<T>,
    inner_guard: InnerGuard,
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

impl<'a, T: fmt::Debug> fmt::Debug for RwLockReadGuard<'a, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

/// A guard that provides mutable data access.
pub struct RwLockWriteGuard<'a, T> {
    inner: &'a RwLock<T>,
    inner_guard: InnerGuard,
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
        self.inner.lock.fetch_and(!WRITER, Release);
    }
}

impl<'a, T: fmt::Debug> fmt::Debug for RwLockWriteGuard<'a, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

/// A guard that provides immutable data access but can be atomically
/// upgraded to `RwLockWriteGuard`.
pub struct RwLockUpgradeableGuard<'a, T> {
    inner: &'a RwLock<T>,
    inner_guard: InnerGuard,
}

impl<'a, T> RwLockUpgradeableGuard<'a, T> {
    /// Upgrade this upread guard to a write guard atomically.
    ///
    /// After calling this method, subsequent readers will be blocked
    /// while previous readers remain unaffected. The calling thread
    /// will spin-wait until previous readers finish.
    pub fn upgrade(mut self) -> RwLockWriteGuard<'a, T> {
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
    /// This function will never spin-wait and will return immediately.
    pub fn try_upgrade(mut self) -> Result<RwLockWriteGuard<'a, T>, Self> {
        let inner = self.inner;
        let res = self.inner.lock.compare_exchange(
            UPGRADEABLE_READER | BEING_UPGRADED,
            WRITER | UPGRADEABLE_READER,
            AcqRel,
            Relaxed,
        );
        if res.is_ok() {
            let inner_guard = match &mut self.inner_guard {
                InnerGuard::IrqGuard(irq_guard) => InnerGuard::IrqGuard(irq_guard.transfer_to()),
                InnerGuard::PreemptGuard(preempt_guard) => {
                    InnerGuard::PreemptGuard(preempt_guard.transfer_to())
                }
            };
            drop(self);
            Ok(RwLockWriteGuard { inner, inner_guard })
        } else {
            Err(self)
        }
    }
}

impl<'a, T> Deref for RwLockUpgradeableGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.inner.val.get() }
    }
}

impl<'a, T> Drop for RwLockUpgradeableGuard<'a, T> {
    fn drop(&mut self) {
        self.inner.lock.fetch_sub(UPGRADEABLE_READER, Release);
    }
}

impl<'a, T: fmt::Debug> fmt::Debug for RwLockUpgradeableGuard<'a, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}
