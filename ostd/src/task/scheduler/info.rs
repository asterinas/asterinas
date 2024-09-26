// SPDX-License-Identifier: MPL-2.0

//! Scheduling related information in a task.

use core::sync::atomic::{AtomicU32, Ordering};

use crate::task::Task;

/// Fields of a task that OSTD will never touch.
///
/// The type ought to be defined by the OSTD user and injected into the task.
/// They are not part of the dynamic task data because it's slower there for
/// the user-defined scheduler to access. The better ways to let the user
/// define them, such as
/// [existential types](https://github.com/rust-lang/rfcs/pull/2492) do not
/// exist yet. So we decide to define them in OSTD.
pub struct TaskScheduleInfo {
    /// The CPU that the task would like to be running on.
    pub cpu: AtomicCpuId,
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

impl CommonSchedInfo for Task {
    fn cpu(&self) -> &AtomicCpuId {
        &self.schedule_info().cpu
    }
}

/// Trait for fetching common scheduling information.
pub trait CommonSchedInfo {
    /// Gets the CPU that the task is running on or lately ran on.
    fn cpu(&self) -> &AtomicCpuId;
}
