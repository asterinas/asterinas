// SPDX-License-Identifier: MPL-2.0

//! Tasks are the unit of code execution.

mod nice;
mod preempt;
mod priority;
mod processor;
mod scheduler;
#[allow(clippy::module_inception)]
mod task;

pub use self::nice::Nice;
pub use self::preempt::{in_atomic, preemptible, DisablePreemptGuard};
pub use self::priority::Priority;
pub use self::processor::{current_task, schedule, switch_to, yield_now};
pub use self::scheduler::{add_task, remove_task, set_scheduler, Scheduler, TaskNumber};
pub use self::task::{
    NeedResched, ReadPriority, Task, TaskAdapter, TaskOptions, TaskStatus, WakeUp, WritePriority,
};

pub fn init() {
    self::processor::init();
    self::scheduler::init();
}
