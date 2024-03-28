// SPDX-License-Identifier: MPL-2.0

pub mod nice;
use alloc::boxed::Box;

use aster_frame::task::{set_global_scheduler, set_local_scheduler};

use self::priority_scheduler::{PreemptGlobalScheduler, PreemptLocalScheduler};

mod priority_scheduler;

pub fn init_global_scheduler() {
    let preempt_scheduler = Box::new(PreemptGlobalScheduler::new());
    let scheduler = Box::<PreemptGlobalScheduler>::leak(preempt_scheduler);
    set_global_scheduler(scheduler);
}

pub fn init_local_scheduler() {
    let preempt_scheduler = Box::new(PreemptLocalScheduler::new());
    let scheduler = Box::<PreemptLocalScheduler>::leak(preempt_scheduler);
    set_local_scheduler(scheduler);
}
