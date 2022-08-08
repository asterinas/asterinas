/// A spin lock.
pub struct SpinLock<T: ?Sized> {
   val: T, 
}

impl<T> SpinLock<T> {
    /// Creates a new spin lock.
    pub fn new(val: T) -> Self {
        todo!()
    }

    /// Acquire the spin lock. 
    /// 
    /// This method runs in a busy loop until the lock can be acquired.
    /// After acquiring the spin lock, all interrupts are disabled.
    pub fn lock(&self) -> SpinLockGuard<'a> {
        todo!()
    }
}

unsafe impl<T: ?Sized + Send> Send for SpinLock<T> {}
unsafe impl<T: ?Sized + Send> Sync for SpinLock<T> {}

/// The guard of a spin lock.
pub struct SpinLockGuard<'a, T: ?Sized + 'a> {
    lock: &'a SpinLock<T>
}

impl<'a, T> Deref for SpinLockGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &T {
        todo!()
    }
}

impl<'a, T: ?Sized> !Send for SpinLockGuard<'a, T> {}

unsafe impl<T: ?Sized + Sync> Sync for SpinLockGuard<'_, T> {}