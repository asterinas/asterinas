// SPDX-License-Identifier: MPL-2.0

//! Tasks are the unit of code execution.

pub(crate) mod atomic_mode;
mod preempt;
mod processor;
pub mod scheduler;
#[allow(clippy::module_inception)]
mod task;

pub(crate) use preempt::cpu_local::reset_preempt_info;

pub use self::{
    preempt::{disable_preempt, DisabledPreemptGuard},
    task::{AtomicCpuId, Priority, Task, TaskAdapter, TaskContextApi, TaskOptions},
};
