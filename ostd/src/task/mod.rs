// SPDX-License-Identifier: MPL-2.0

//! Tasks are the unit of code execution.

mod processor;
pub mod scheduler;
#[allow(clippy::module_inception)]
mod task;

pub use self::{
    processor::{disable_preempt, DisablePreemptGuard},
    task::{AtomicCpuId, Priority, Task, TaskAdapter, TaskContextApi, TaskOptions},
};
