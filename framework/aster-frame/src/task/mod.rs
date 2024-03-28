// SPDX-License-Identifier: MPL-2.0

//! Tasks are the unit of code execution.

pub mod atomic;
mod priority;
mod processor;
mod scheduler;

#[allow(clippy::module_inception)]
mod task;

pub use self::{
    priority::Priority,
    processor::{current_task, preempt, schedule},
    scheduler::{add_task, set_scheduler, Scheduler},
    task::{Task, TaskAdapter, TaskOptions, TaskStatus},
};
