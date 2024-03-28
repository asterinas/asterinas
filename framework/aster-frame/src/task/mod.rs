// SPDX-License-Identifier: MPL-2.0

//! Tasks are the unit of code execution.

mod atomic;
mod priority;
mod processor;
mod scheduler;
#[allow(clippy::module_inception)]
mod task;

pub use self::{
    atomic::{enter_atomic_mode, might_break_atomic_mode, AtomicModeGuard},
    priority::Priority,
    processor::{current_task, disable_preempt, preempt, schedule, DisablePreemptGuard},
    scheduler::{add_task, set_scheduler, FifoScheduler, Scheduler},
    task::{Task, TaskAdapter, TaskContextApi, TaskOptions, TaskStatus},
};
