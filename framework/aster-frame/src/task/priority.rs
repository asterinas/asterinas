// SPDX-License-Identifier: MPL-2.0

use crate::config::REAL_TIME_TASK_PRI;

/// The priority of a task.
/// Similar to Linux, a larger value represents a lower priority,
/// with a range of 0 to 139. Priorities ranging from 0 to 99 are considered real-time,
/// while those ranging from 100 to 139 are considered normal.
#[derive(Copy, Clone)]
pub struct Priority(u16);

impl Priority {
    pub const fn new(val: u16) -> Self {
        assert!(val <= 139);
        Self(val)
    }

    pub const fn lowest() -> Self {
        Self::new(139)
    }

    pub const fn low() -> Self {
        Self::new(110)
    }

    pub const fn normal() -> Self {
        Self::new(100)
    }

    pub const fn high() -> Self {
        Self::new(10)
    }

    pub const fn highest() -> Self {
        Self::new(0)
    }

    pub const fn set(&mut self, val: u16) {
        self.0 = val;
    }

    pub const fn get(self) -> u16 {
        self.0
    }

    pub const fn is_real_time(&self) -> bool {
        self.0 < REAL_TIME_TASK_PRI
    }
}
