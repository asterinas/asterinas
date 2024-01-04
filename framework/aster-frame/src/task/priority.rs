use super::Nice;
use crate::config::REAL_TIME_TASK_PRI;

/// The priority of a task.
/// Similar to Linux, a larger value represents a lower priority,
/// with a range of 0 to 139. Priorities ranging from 0 to 99 are considered real-time,
/// while those ranging from 100 to 139 are considered normal.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
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

    /// Convert static priority [ MAX_RT_PRIO..MAX_PRIO ]
    /// to user-nice values [ -20 ... 0 ... 19 ]
    pub const fn as_nice(&self) -> Option<i8> {
        if self.is_real_time() {
            None
        } else {
            Some((self.0 as i8 - Priority::normal().get() as i8) - 20)
        }
    }
}

impl PartialOrd for Priority {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Priority {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.0.cmp(&other.0).reverse()
    }
}

impl From<Nice> for Priority {
    /// Convert user-nice values [ -20 ... 0 ... 19 ]
    /// to static priority [ MAX_RT_PRIO..MAX_PRIO ]
    fn from(nice: Nice) -> Self {
        let prio = (nice - 20) + Priority::normal().get() as i8;
        Self::new(prio as u16)
    }
}
