// SPDX-License-Identifier: MPL-2.0

pub const REAL_TIME_TASK_PRIORITY: u16 = 100;

/// The priority of a task.
///
/// Similar to Linux, a larger value represents a lower priority,
/// with a range of 0 to 139. Priorities ranging from 0 to 99 are considered real-time,
/// while those ranging from 100 to 139 are considered normal.
#[derive(Copy, Clone)]
pub struct Priority(u16);

impl Priority {
    /// Creates a new `Priority` with the specified value.
    ///
    /// # Panics
    ///
    /// Panics if the `val` is greater than 139.
    pub const fn new(val: u16) -> Self {
        assert!(val <= 139);
        Self(val)
    }

    /// Returns a `Priority` representing the lowest priority (139).
    pub const fn lowest() -> Self {
        Self::new(139)
    }

    /// Returns a `Priority` representing a low priority.
    pub const fn low() -> Self {
        Self::new(110)
    }

    /// Returns a `Priority` representing a normal priority.
    pub const fn normal() -> Self {
        Self::new(100)
    }

    /// Returns a `Priority` representing a high priority.
    pub const fn high() -> Self {
        Self::new(10)
    }

    /// Returns a `Priority` representing the highest priority (0).
    pub const fn highest() -> Self {
        Self::new(0)
    }

    /// Sets the value of the `Priority`.
    pub const fn set(&mut self, val: u16) {
        self.0 = val;
    }

    /// Returns the value of the `Priority`.
    pub const fn get(self) -> u16 {
        self.0
    }

    /// Checks if the `Priority` is considered a real-time priority.
    pub const fn is_real_time(&self) -> bool {
        self.0 < REAL_TIME_TASK_PRIORITY
    }
}
