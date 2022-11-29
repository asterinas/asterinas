use core::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
};

#[derive(Debug, Default)]
#[repr(transparent)]
pub struct Cell<T>(UnsafeCell<T>);

unsafe impl<T> Sync for Cell<T> {}

impl<T> Cell<T> {
    /// User is responsible to guarantee that inner struct is only used in
    /// uniprocessor.
    #[inline(always)]
    pub const fn new(val: T) -> Self {
        Self(UnsafeCell::new(val))
    }

    #[inline(always)]
    pub fn get(&self) -> &mut T {
        unsafe { &mut *self.0.get() }
    }
}

impl<T> Deref for Cell<T> {
    type Target = T;
    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

impl<T> DerefMut for Cell<T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.get()
    }
}
