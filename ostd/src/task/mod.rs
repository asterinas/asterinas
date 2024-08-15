// SPDX-License-Identifier: MPL-2.0

//! Tasks are the unit of code execution.

mod preempt;
mod processor;
pub mod scheduler;
#[allow(clippy::module_inception)]
mod task;

pub use self::{
    preempt::{disable_preempt, DisablePreemptGuard},
    task::{AtomicCpuId, Priority, Task, TaskAdapter, TaskContextApi, TaskOptions},
};
