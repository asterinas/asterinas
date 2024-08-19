// SPDX-License-Identifier: MPL-2.0

use core::ops::MulAssign;

use super::prelude::*;

/// Returns the current time.
pub fn now() -> Duration {
    crate::time::clocks::RealTimeCoarseClock::get().read_time()
}

pub trait IsPowerOf: Copy + Sized + MulAssign + PartialOrd {
    /// Returns true if and only if `self == x^k` for some `k` where `k > 0`.
    ///
    /// The `x` must be a positive value.
    fn is_power_of(&self, x: Self) -> bool {
        let mut power = x;
        while power < *self {
            power *= x;
        }

        power == *self
    }
}

macro_rules! impl_ipo_for {
    ($($ipo_ty:ty),*) => {
        $(impl IsPowerOf for $ipo_ty {})*
    };
}

impl_ipo_for!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128, isize, usize);

/// The `Dirty` wraps a value of type `T` with functions similar to that of a rw-lock,
/// but simply sets a dirty flag on `write()`.
pub struct Dirty<T: Debug> {
    value: T,
    dirty: bool,
}

impl<T: Debug> Dirty<T> {
    /// Creates a new Dirty without setting the dirty flag.
    pub fn new(val: T) -> Dirty<T> {
        Dirty {
            value: val,
            dirty: false,
        }
    }

    /// Creates a new Dirty with setting the dirty flag.
    pub fn new_dirty(val: T) -> Dirty<T> {
        Dirty {
            value: val,
            dirty: true,
        }
    }

    /// Returns true if dirty, false otherwise.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Clears the dirty flag.
    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }
}

impl<T: Debug> Deref for Dirty<T> {
    type Target = T;

    /// Returns the imutable value.
    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T: Debug> DerefMut for Dirty<T> {
    /// Returns the mutable value, sets the dirty flag.
    fn deref_mut(&mut self) -> &mut T {
        self.dirty = true;
        &mut self.value
    }
}

impl<T: Debug> Drop for Dirty<T> {
    /// Guards if it is dirty when dropping.
    fn drop(&mut self) {
        if self.is_dirty() {
            warn!("[{:?}] is dirty then dropping", self.value);
        }
    }
}

impl<T: Debug> Debug for Dirty<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        let tag = if self.dirty { "Dirty" } else { "Clean" };
        write!(f, "[{}] {:?}", tag, self.value)
    }
}
