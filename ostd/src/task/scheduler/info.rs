// SPDX-License-Identifier: MPL-2.0

//! Scheduling related information in a task.

use core::sync::atomic::{AtomicU32, Ordering};

use crate::{cpu::CpuId, task::Task};

/// Fields of a task that OSTD will never touch.
///
/// The type ought to be defined by the OSTD user and injected into the task.
/// They are not part of the dynamic task data because it's slower there for
/// the user-defined scheduler to access. The better ways to let the user
/// define them, such as
/// [existential types](https://github.com/rust-lang/rfcs/pull/2492) do not
/// exist yet. So we decide to define them in OSTD.
#[derive(Debug)]
pub struct TaskScheduleInfo {
    /// The CPU that the task would like to be running on.
    pub cpu: AtomicCpuId,
}

/// An atomic CPUID container.
#[derive(Debug)]
pub struct AtomicCpuId(AtomicU32);

impl AtomicCpuId {
    /// The null value of CPUID.
    ///
    /// An `AtomicCpuId` with `AtomicCpuId::NONE` as its inner value is empty.
    const NONE: u32 = u32::MAX;

    /// Sets the inner value of an `AtomicCpuId` if it's empty.
    ///
    /// The return value is a result indicating whether the new value was written
    /// and containing the previous value. If the previous value is empty, it returns
    /// `Ok(())`. Otherwise, it returns `Err(previous_value)` which the previous
    /// value is a valid CPU ID.
    pub fn set_if_is_none(&self, cpu_id: CpuId) -> core::result::Result<(), CpuId> {
        self.0
            .compare_exchange(
                Self::NONE,
                cpu_id.as_usize() as u32,
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .map(|_| ())
            .map_err(|prev| (prev as usize).try_into().unwrap())
    }

    /// Sets the inner value of an `AtomicCpuId` to `AtomicCpuId::NONE`, i.e. makes
    /// an `AtomicCpuId` empty.
    pub fn set_to_none(&self) {
        self.0.store(Self::NONE, Ordering::Relaxed);
    }

    /// Gets the inner value of an `AtomicCpuId`.
    pub fn get(&self) -> Option<CpuId> {
        let val = self.0.load(Ordering::Relaxed);
        if val == Self::NONE {
            None
        } else {
            Some((val as usize).try_into().ok()?)
        }
    }
}

impl Default for AtomicCpuId {
    fn default() -> Self {
        Self(AtomicU32::new(Self::NONE))
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
