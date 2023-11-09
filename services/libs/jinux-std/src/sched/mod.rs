mod scheduler;

use crate::prelude::*;
use jinux_frame::task::{set_scheduler, Scheduler};
use scheduler::vanilla::PreemptScheduler;
// use scheduler::multiqueue::MultiQueueScheduler;

// There may be multiple scheduling policies in the system,
// and subsequent schedulers can be placed under this module.
pub fn init() {
    let preempt_scheduler = Box::new(PreemptScheduler::new());
    let scheduler = Box::<PreemptScheduler>::leak(preempt_scheduler);
    set_scheduler(scheduler);
}

// todo: add a scheduler manager to manage multiple schedulers
