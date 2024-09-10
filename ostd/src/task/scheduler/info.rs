// SPDX-License-Identifier: MPL-2.0

//! Scheduling related information in a task.

use core::sync::atomic::{AtomicU32, Ordering};

use crate::cpu::CpuSet;

/// Fields of a task that OSTD will never touch.
///
/// The type ought to be defined by the OSTD user and injected into the task.
/// They are not part of the dynamic task data because it's slower there for
/// the user-defined scheduler to access. The better ways to let the user
/// define them, such as
/// [existential types](https://github.com/rust-lang/rfcs/pull/2492) do not
/// exist yet. So we decide to define them in OSTD.
pub struct TaskScheduleInfo {
    /// Priority of the task.
    pub priority: Priority,
    /// The CPU that the task would like to be running on.
    pub cpu: AtomicCpuId,
    /// The CPUs that this task can run on.
    pub cpu_affinity: CpuSet,
}

/// The priority of a real-time task.
pub const REAL_TIME_TASK_PRIORITY: u16 = 100;

/// The priority of a task.
///
/// Similar to Linux, a larger value represents a lower priority,
/// with a range of 0 to 139. Priorities ranging from 0 to 99 are considered real-time,
/// while those ranging from 100 to 139 are considered normal.
#[derive(Copy, Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct Priority(u16);

impl Priority {
    const LOWEST: u16 = 139;
    const LOW: u16 = 110;
    const NORMAL: u16 = 100;
    const HIGH: u16 = 10;
    const HIGHEST: u16 = 0;

    /// Creates a new `Priority` with the specified value.
    ///
    /// # Panics
    ///
    /// Panics if the `val` is greater than 139.
    pub const fn new(val: u16) -> Self {
        assert!(val <= Self::LOWEST);
        Self(val)
    }

    /// Returns a `Priority` representing the lowest priority (139).
    pub const fn lowest() -> Self {
        Self::new(Self::LOWEST)
    }

    /// Returns a `Priority` representing a low priority.
    pub const fn low() -> Self {
        Self::new(Self::LOW)
    }

    /// Returns a `Priority` representing a normal priority.
    pub const fn normal() -> Self {
        Self::new(Self::NORMAL)
    }

    /// Returns a `Priority` representing a high priority.
    pub const fn high() -> Self {
        Self::new(Self::HIGH)
    }

    /// Returns a `Priority` representing the highest priority (0).
    pub const fn highest() -> Self {
        Self::new(Self::HIGHEST)
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

/// An atomic CPUID container.
pub struct AtomicCpuId(AtomicU32);

impl AtomicCpuId {
    /// The null value of CPUID.
    ///
    /// An `AtomicCpuId` with `AtomicCpuId::NONE` as its inner value is empty.
    const NONE: u32 = u32::MAX;

    fn new(cpu_id: u32) -> Self {
        Self(AtomicU32::new(cpu_id))
    }

    /// Sets the inner value of an `AtomicCpuId` if it's empty.
    ///
    /// The return value is a result indicating whether the new value was written
    /// and containing the previous value.
    pub fn set_if_is_none(&self, cpu_id: u32) -> core::result::Result<u32, u32> {
        self.0
            .compare_exchange(Self::NONE, cpu_id, Ordering::Relaxed, Ordering::Relaxed)
    }

    /// Sets the inner value of an `AtomicCpuId` to `AtomicCpuId::NONE`, i.e. makes
    /// an `AtomicCpuId` empty.
    pub fn set_to_none(&self) {
        self.0.store(Self::NONE, Ordering::Relaxed);
    }

    /// Gets the inner value of an `AtomicCpuId`.
    pub fn get(&self) -> Option<u32> {
        let val = self.0.load(Ordering::Relaxed);
        if val == Self::NONE {
            None
        } else {
            Some(val)
        }
    }
}

impl Default for AtomicCpuId {
    fn default() -> Self {
        Self::new(Self::NONE)
    }
}
