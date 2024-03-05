// SPDX-License-Identifier: MPL-2.0

//! Tasks are the unit of code execution.

mod preempt;
mod priority;
mod processor;
mod scheduler;
#[allow(clippy::module_inception)]
mod task;

pub use self::{
    preempt::{in_atomic, is_preemptible, DisablePreemptGuard},
    priority::Priority,
    processor::{current_task, schedule, yield_now, yield_to},
    scheduler::{add_task, clear_task, set_scheduler, Scheduler},
    task::{
        Current, NeedResched, ReadPriority, SchedTaskBase, Task, TaskAdapter, TaskOptions,
        TaskStatus, WakeUp,
    },
};

pub fn init() {
    self::processor::init();
    self::scheduler::init();
}
