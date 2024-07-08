// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicI8, Ordering};

use bytemuck_derive::{Pod, Zeroable};

#[repr(transparent)]
#[derive(Copy, Clone, Debug, PartialEq, PartialOrd, Pod, Zeroable)]
pub struct GenericPriority<const MIN: i8, const MAX: i8>(i8);

impl<const MIN: i8, const MAX: i8> GenericPriority<MIN, MAX> {
    pub const MIN: Self = Self::new(MIN);
    pub const MAX: Self = Self::new(MAX);

    /// Creates a new `Priority` with the specified value.
    pub const fn new(val: i8) -> Self {
        assert!(val <= MAX && val >= MIN);
        Self(val)
    }

    /// Sets the value of the `Priority`.
    pub const fn set(&mut self, val: i8) {
        self.0 = val;
    }

    /// Returns the value of the `Priority`.
    pub const fn get(self) -> i8 {
        self.0
    }
}

/// The kernel-internal scheduling priority.
///
/// Lower numeric value means higher priority.
/// Formula:
/// - Priority = Nice + 20 for normal tasks.
/// - Priority = -1 - SchedPriority for real-time tasks.
pub type Priority = GenericPriority<-100, 39>;

impl Priority {
    pub const DEFAULT_PTHREAD_PRIORITY: Self = Self::new(20);
    pub const DEFAULT_NORMAL_KTHREAD_PRIORITY: Self = Self::new(0);
    pub const DEFAULT_RT_KTHREAD_PRIORITY: Self = Self::new(-51);
}

impl From<Nice> for Priority {
    fn from(value: Nice) -> Self {
        Self::new(value.get() + 20)
    }
}

/// The process scheduling nice value.
///
/// The nice value is an attribute that can be used to influence the
/// CPU scheduler to favor or disfavor a process in scheduling decisions.
///
/// It is a value in the range -20 to 19, with -20 being the highest priority
/// and 19 being the lowest priority. The smaller values give a process a higher
/// scheduling priority.
pub type Nice = GenericPriority<-20, 19>;

impl Default for Nice {
    fn default() -> Self {
        Self::new(0)
    }
}

impl From<Priority> for Nice {
    fn from(value: Priority) -> Self {
        assert!(value.get() >= 0);
        Self::new(value.get() - 20)
    }
}

/// The POSIX.1 sched_priority.
///
/// Used for POSIX.1's `sched_get_priority_max/min`, `sched_setscheduler`.
/// For Linux-compatibility, all normal tasks has a sched_priority
/// of 0 while real-time tasks' sched_priority can vary from 1 to 99.
type SchedPriority = GenericPriority<0, 99>;

impl From<Priority> for SchedPriority {
    fn from(value: Priority) -> Self {
        assert_ne!(value.get(), -1);
        if value.get() >= 0 {
            Self::new(0)
        } else {
            Self::new(-1 - value.get())
        }
    }
}

/// A `Priority` which can be safely shared between threads.
#[derive(Debug)]
pub struct AtomicPriority(AtomicI8);

impl AtomicPriority {
    /// Creates a new atomic priority.
    pub fn new(status: Priority) -> Self {
        Self(AtomicI8::new(status.get()))
    }

    /// Loads a value from the atomic priority.
    pub fn load(&self, order: Ordering) -> Priority {
        Priority::new(self.0.load(order))
    }

    /// Stores a value into the atomic priority.
    pub fn store(&self, new_priority: Priority, order: Ordering) {
        self.0.store(new_priority.get(), order);
    }

    /// Stores a value into the atomic priority if the current value is the same as the `current` value.
    pub fn compare_exchange(
        &self,
        current: Priority,
        new: Priority,
        success: Ordering,
        failure: Ordering,
    ) -> Result<Priority, Priority> {
        self.0
            .compare_exchange(current.get(), new.get(), success, failure)
            .map(Priority::new)
            .map_err(Priority::new)
    }
}
