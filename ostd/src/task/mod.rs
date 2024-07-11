// SPDX-License-Identifier: MPL-2.0

//! Tasks are the unit of code execution.

mod processor;
mod scheduler;
#[allow(clippy::module_inception)]
mod task;

pub use self::{
    processor::{current_task, disable_preempt, DisablePreemptGuard},
    scheduler::{
        add_task, inject_scheduler, schedule, yield_now, EnqueueFlags, FifoScheduler,
        LocalRunQueue, Scheduler, UpdateFlags, YieldFlags,
    },
    task::{Priority, Task, TaskAdapter, TaskContextApi, TaskOptions, TaskStatus},
};
