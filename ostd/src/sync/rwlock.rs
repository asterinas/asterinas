// SPDX-License-Identifier: MPL-2.0

use core::{
    cell::UnsafeCell,
    fmt,
    marker::PhantomData,
    ops::{Deref, DerefMut},
    sync::atomic::{
        AtomicUsize,
        Ordering::{AcqRel, Acquire, Relaxed, Release},
    },
};

use super::{
    guard::{GuardTransfer, SpinGuardian},
    PreemptDisabled,
};
use crate::task::atomic_mode::AsAtomicModeGuard;

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
/// In addition to traditional read and write locks, this implementation
/// provides the upgradeable read lock (`upread lock`). The `upread lock`
/// can be upgraded to write locks atomically, useful in scenarios
/// where a decision to write is made after reading.
///
/// The type parameter `T` represents the data that this lock is protecting.
/// It is necessary for `T` to satisfy [`Send`] to be shared across tasks and
/// [`Sync`] to permit concurrent access via readers. The [`Deref`] method (and
/// [`DerefMut`] for the writer) is implemented for the RAII guards returned
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
/// # About Guard
///
/// See the comments of [`SpinLock`].
///
/// # Examples
///
/// ```
/// use ostd::sync::RwLock;
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
///
/// [`SpinLock`]: super::SpinLock
pub struct RwLock<T: ?Sized, Guard = PreemptDisabled> {
    guard: PhantomData<Guard>,
    /// The internal representation of the lock state is as follows:
    /// - **Bit 63:** Writer lock.
    /// - **Bit 62:** Upgradeable reader lock.
    /// - **Bit 61:** Indicates if an upgradeable reader is being upgraded.
    /// - **Bits 60-0:** Reader lock count.
    lock: AtomicUsize,
    val: UnsafeCell<T>,
}

const READER: usize = 1;
const WRITER: usize = 1 << (usize::BITS - 1);
const UPGRADEABLE_READER: usize = 1 << (usize::BITS - 2);
const BEING_UPGRADED: usize = 1 << (usize::BITS - 3);
const MAX_READER: usize = 1 << (usize::BITS - 4);

impl<T, G> RwLock<T, G> {
    /// Creates a new spin-based read-write lock with an initial value.
    pub const fn new(val: T) -> Self {
        Self {
            guard: PhantomData,
            lock: AtomicUsize::new(0),
            val: UnsafeCell::new(val),
        }
    }
}

impl<T: ?Sized, G: SpinGuardian> RwLock<T, G> {
    /// Acquires a read lock and spin-wait until it can be acquired.
    ///
    /// The calling thread will spin-wait until there are no writers or
    /// upgrading upreaders present. There is no guarantee for the order
    /// in which other readers or writers waiting simultaneously will
    /// obtain the lock.
    pub fn read(&self) -> RwLockReadGuard<T, G> {
        loop {
            if let Some(readguard) = self.try_read() {
                return readguard;
            } else {
                core::hint::spin_loop();
            }
        }
    }

    /// Acquires a write lock and spin-wait until it can be acquired.
    ///
    /// The calling thread will spin-wait until there are no other writers,
    /// upreaders or readers present. There is no guarantee for the order
    /// in which other readers or writers waiting simultaneously will
    /// obtain the lock.
    pub fn write(&self) -> RwLockWriteGuard<T, G> {
        loop {
            if let Some(writeguard) = self.try_write() {
                return writeguard;
            } else {
                core::hint::spin_loop();
            }
        }
    }

    /// Acquires an upreader and spin-wait until it can be acquired.
    ///
    /// The calling thread will spin-wait until there are no other writers,
    /// or upreaders. There is no guarantee for the order in which other
    /// readers or writers waiting simultaneously will obtain the lock.
    ///
    /// Upreader will not block new readers until it tries to upgrade. Upreader
    /// and reader do not differ before invoking the upgread method. However,
    /// only one upreader can exist at any time to avoid deadlock in the
    /// upgread method.
    pub fn upread(&self) -> RwLockUpgradeableGuard<T, G> {
        loop {
            if let Some(guard) = self.try_upread() {
                return guard;
            } else {
                core::hint::spin_loop();
            }
        }
    }

    /// Attempts to acquire a read lock.
    ///
    /// This function will never spin-wait and will return immediately.
    pub fn try_read(&self) -> Option<RwLockReadGuard<T, G>> {
        let guard = G::read_guard();
        let lock = self.lock.fetch_add(READER, Acquire);
        if lock & (WRITER | MAX_READER | BEING_UPGRADED) == 0 {
            Some(RwLockReadGuard { inner: self, guard })
        } else {
            self.lock.fetch_sub(READER, Release);
            None
        }
    }

    /// Attempts to acquire a write lock.
    ///
    /// This function will never spin-wait and will return immediately.
    pub fn try_write(&self) -> Option<RwLockWriteGuard<T, G>> {
        let guard = G::guard();
        if self
            .lock
            .compare_exchange(0, WRITER, Acquire, Relaxed)
            .is_ok()
        {
            Some(RwLockWriteGuard { inner: self, guard })
        } else {
            None
        }
    }

    /// Attempts to acquire an upread lock.
    ///
    /// This function will never spin-wait and will return immediately.
    pub fn try_upread(&self) -> Option<RwLockUpgradeableGuard<T, G>> {
        let guard = G::guard();
        let lock = self.lock.fetch_or(UPGRADEABLE_READER, Acquire) & (WRITER | UPGRADEABLE_READER);
        if lock == 0 {
            return Some(RwLockUpgradeableGuard { inner: self, guard });
        } else if lock == WRITER {
            self.lock.fetch_sub(UPGRADEABLE_READER, Release);
        }
        None
    }

    /// Returns a mutable reference to the underlying data.
    ///
    /// This method is zero-cost: By holding a mutable reference to the lock, the compiler has
    /// already statically guaranteed that access to the data is exclusive.
    pub fn get_mut(&mut self) -> &mut T {
        self.val.get_mut()
    }

    /// Returns a raw pointer to the underlying data.
    ///
    /// This method is safe, but it's up to the caller to ensure that access to the data behind it
    /// is still safe.
    pub(super) fn as_ptr(&self) -> *mut T {
        self.val.get()
    }
}

impl<T: ?Sized + fmt::Debug, G> fmt::Debug for RwLock<T, G> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.val, f)
    }
}

/// Because there can be more than one readers to get the T's immutable ref,
/// so T must be Sync to guarantee the sharing safety.
unsafe impl<T: ?Sized + Send, G> Send for RwLock<T, G> {}
unsafe impl<T: ?Sized + Send + Sync, G> Sync for RwLock<T, G> {}

impl<T: ?Sized, R: Deref<Target = RwLock<T, G>> + Clone, G: SpinGuardian> !Send
    for RwLockWriteGuard_<T, R, G>
{
}
unsafe impl<T: ?Sized + Sync, R: Deref<Target = RwLock<T, G>> + Clone + Sync, G: SpinGuardian> Sync
    for RwLockWriteGuard_<T, R, G>
{
}

impl<T: ?Sized, R: Deref<Target = RwLock<T, G>> + Clone, G: SpinGuardian> !Send
    for RwLockReadGuard_<T, R, G>
{
}
unsafe impl<T: ?Sized + Sync, R: Deref<Target = RwLock<T, G>> + Clone + Sync, G: SpinGuardian> Sync
    for RwLockReadGuard_<T, R, G>
{
}

impl<T: ?Sized, R: Deref<Target = RwLock<T, G>> + Clone, G: SpinGuardian> !Send
    for RwLockUpgradeableGuard_<T, R, G>
{
}
unsafe impl<T: ?Sized + Sync, R: Deref<Target = RwLock<T, G>> + Clone + Sync, G: SpinGuardian> Sync
    for RwLockUpgradeableGuard_<T, R, G>
{
}

/// A guard that provides immutable data access.
#[clippy::has_significant_drop]
#[must_use]
pub struct RwLockReadGuard_<T: ?Sized, R: Deref<Target = RwLock<T, G>> + Clone, G: SpinGuardian> {
    guard: G::ReadGuard,
    inner: R,
}

impl<T: ?Sized, R: Deref<Target = RwLock<T, G>> + Clone, G: SpinGuardian> AsAtomicModeGuard
    for RwLockReadGuard_<T, R, G>
{
    fn as_atomic_mode_guard(&self) -> &dyn crate::task::atomic_mode::InAtomicMode {
        self.guard.as_atomic_mode_guard()
    }
}

/// A guard that provides shared read access to the data protected by a [`RwLock`].
pub type RwLockReadGuard<'a, T, G> = RwLockReadGuard_<T, &'a RwLock<T, G>, G>;

impl<T: ?Sized, R: Deref<Target = RwLock<T, G>> + Clone, G: SpinGuardian> Deref
    for RwLockReadGuard_<T, R, G>
{
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.inner.val.get() }
    }
}

impl<T: ?Sized, R: Deref<Target = RwLock<T, G>> + Clone, G: SpinGuardian> Drop
    for RwLockReadGuard_<T, R, G>
{
    fn drop(&mut self) {
        self.inner.lock.fetch_sub(READER, Release);
    }
}

impl<T: ?Sized + fmt::Debug, R: Deref<Target = RwLock<T, G>> + Clone, G: SpinGuardian> fmt::Debug
    for RwLockReadGuard_<T, R, G>
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

/// A guard that provides mutable data access.
pub struct RwLockWriteGuard_<T: ?Sized, R: Deref<Target = RwLock<T, G>> + Clone, G: SpinGuardian> {
    guard: G::Guard,
    inner: R,
}

impl<T: ?Sized, R: Deref<Target = RwLock<T, G>> + Clone, G: SpinGuardian> AsAtomicModeGuard
    for RwLockWriteGuard_<T, R, G>
{
    fn as_atomic_mode_guard(&self) -> &dyn crate::task::atomic_mode::InAtomicMode {
        self.guard.as_atomic_mode_guard()
    }
}

/// A guard that provides exclusive write access to the data protected by a [`RwLock`].
pub type RwLockWriteGuard<'a, T, G> = RwLockWriteGuard_<T, &'a RwLock<T, G>, G>;

impl<T: ?Sized, R: Deref<Target = RwLock<T, G>> + Clone, G: SpinGuardian> Deref
    for RwLockWriteGuard_<T, R, G>
{
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.inner.val.get() }
    }
}

impl<T: ?Sized, R: Deref<Target = RwLock<T, G>> + Clone, G: SpinGuardian>
    RwLockWriteGuard_<T, R, G>
{
    /// Atomically downgrades a write guard to an upgradeable reader guard.
    ///
    /// This method always succeeds because the lock is exclusively held by the writer.
    pub fn downgrade(mut self) -> RwLockUpgradeableGuard_<T, R, G> {
        loop {
            self = match self.try_downgrade() {
                Ok(guard) => return guard,
                Err(e) => e,
            };
        }
    }

    /// This is not exposed as a public method to prevent intermediate lock states from affecting the
    /// downgrade process.
    fn try_downgrade(mut self) -> Result<RwLockUpgradeableGuard_<T, R, G>, Self> {
        let inner = self.inner.clone();
        let res = self
            .inner
            .lock
            .compare_exchange(WRITER, UPGRADEABLE_READER, AcqRel, Relaxed);
        if res.is_ok() {
            let guard = self.guard.transfer_to();
            drop(self);
            Ok(RwLockUpgradeableGuard_ { inner, guard })
        } else {
            Err(self)
        }
    }
}

impl<T: ?Sized, R: Deref<Target = RwLock<T, G>> + Clone, G: SpinGuardian> DerefMut
    for RwLockWriteGuard_<T, R, G>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.inner.val.get() }
    }
}

impl<T: ?Sized, R: Deref<Target = RwLock<T, G>> + Clone, G: SpinGuardian> Drop
    for RwLockWriteGuard_<T, R, G>
{
    fn drop(&mut self) {
        self.inner.lock.fetch_and(!WRITER, Release);
    }
}

impl<T: ?Sized + fmt::Debug, R: Deref<Target = RwLock<T, G>> + Clone, G: SpinGuardian> fmt::Debug
    for RwLockWriteGuard_<T, R, G>
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

/// A guard that provides immutable data access but can be atomically
/// upgraded to `RwLockWriteGuard`.
pub struct RwLockUpgradeableGuard_<
    T: ?Sized,
    R: Deref<Target = RwLock<T, G>> + Clone,
    G: SpinGuardian,
> {
    guard: G::Guard,
    inner: R,
}

impl<T: ?Sized, R: Deref<Target = RwLock<T, G>> + Clone, G: SpinGuardian> AsAtomicModeGuard
    for RwLockUpgradeableGuard_<T, R, G>
{
    fn as_atomic_mode_guard(&self) -> &dyn crate::task::atomic_mode::InAtomicMode {
        self.guard.as_atomic_mode_guard()
    }
}

/// A upgradable guard that provides read access to the data protected by a [`RwLock`].
pub type RwLockUpgradeableGuard<'a, T, G> = RwLockUpgradeableGuard_<T, &'a RwLock<T, G>, G>;

impl<T: ?Sized, R: Deref<Target = RwLock<T, G>> + Clone, G: SpinGuardian>
    RwLockUpgradeableGuard_<T, R, G>
{
    /// Upgrades this upread guard to a write guard atomically.
    ///
    /// After calling this method, subsequent readers will be blocked
    /// while previous readers remain unaffected. The calling thread
    /// will spin-wait until previous readers finish.
    pub fn upgrade(mut self) -> RwLockWriteGuard_<T, R, G> {
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
    pub fn try_upgrade(mut self) -> Result<RwLockWriteGuard_<T, R, G>, Self> {
        let res = self.inner.lock.compare_exchange(
            UPGRADEABLE_READER | BEING_UPGRADED,
            WRITER | UPGRADEABLE_READER,
            AcqRel,
            Relaxed,
        );
        if res.is_ok() {
            let inner = self.inner.clone();
            let guard = self.guard.transfer_to();
            drop(self);
            Ok(RwLockWriteGuard_ { inner, guard })
        } else {
            Err(self)
        }
    }
}

impl<T: ?Sized, R: Deref<Target = RwLock<T, G>> + Clone, G: SpinGuardian> Deref
    for RwLockUpgradeableGuard_<T, R, G>
{
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.inner.val.get() }
    }
}

impl<T: ?Sized, R: Deref<Target = RwLock<T, G>> + Clone, G: SpinGuardian> Drop
    for RwLockUpgradeableGuard_<T, R, G>
{
    fn drop(&mut self) {
        self.inner.lock.fetch_sub(UPGRADEABLE_READER, Release);
    }
}

impl<T: ?Sized + fmt::Debug, R: Deref<Target = RwLock<T, G>> + Clone, G: SpinGuardian> fmt::Debug
    for RwLockUpgradeableGuard_<T, R, G>
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}
