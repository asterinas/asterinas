// SPDX-License-Identifier: MPL-2.0

//! Tasks are the unit of code execution.

mod priority;
mod processor;
mod scheduler;
#[allow(clippy::module_inception)]
mod task;

pub use self::{
    priority::Priority,
    processor::{current_task, disable_preempt, preempt, schedule, DisablePreemptGuard},
    scheduler::{
        add_task_to_global, fetch_task_from_global, preempt_global, set_global_scheduler,
        set_local_scheduler, Scheduler,
    },
    task::{Task, TaskAdapter, TaskOptions, TaskStatus},
};
