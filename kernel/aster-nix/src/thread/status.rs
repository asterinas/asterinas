// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU8, Ordering};

use int_to_c_enum::TryFromInt;

/// A `ThreadStatus` which can be safely shared between threads.
#[derive(Debug)]
pub struct AtomicThreadStatus(AtomicU8);

impl AtomicThreadStatus {
    /// Creates a new atomic status.
    pub fn new(status: ThreadStatus) -> Self {
        Self(AtomicU8::new(status as u8))
    }

    /// Loads a value from the atomic status.
    pub fn load(&self, order: Ordering) -> ThreadStatus {
        ThreadStatus::try_from(self.0.load(order)).unwrap()
    }

    /// Stores a value into the atomic status.
    pub fn store(&self, new_status: ThreadStatus, order: Ordering) {
        self.0.store(new_status as u8, order);
    }

    /// Stores a value into the atomic status if the current value is the same as the `current` value.
    pub fn compare_exchange(
        &self,
        current: ThreadStatus,
        new: ThreadStatus,
        success: Ordering,
        failure: Ordering,
    ) -> Result<ThreadStatus, ThreadStatus> {
        self.0
            .compare_exchange(current as u8, new as u8, success, failure)
            .map(|val| ThreadStatus::try_from(val).unwrap())
            .map_err(|val| ThreadStatus::try_from(val).unwrap())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, TryFromInt)]
#[repr(u8)]
pub enum ThreadStatus {
    Init = 0,
    Running = 1,
    Exited = 2,
    Stopped = 3,
}

impl ThreadStatus {
    pub fn is_running(&self) -> bool {
        *self == ThreadStatus::Running
    }

    pub fn is_exited(&self) -> bool {
        *self == ThreadStatus::Exited
    }

    pub fn is_stopped(&self) -> bool {
        *self == ThreadStatus::Stopped
    }
}
