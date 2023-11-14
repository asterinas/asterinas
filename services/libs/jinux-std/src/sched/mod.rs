mod scheduler;

use crate::prelude::*;
use jinux_frame::task::set_scheduler;
use scheduler::vanilla::PreemptiveFIFOScheduler;
// use scheduler::multiqueue::MultiQueueScheduler;

// There may be multiple scheduling policies in the system,
// and subsequent schedulers can be placed under this module.
pub fn init() {
    let preempt_scheduler = Box::new(PreemptiveFIFOScheduler::new());
    let scheduler = Box::<PreemptiveFIFOScheduler>::leak(preempt_scheduler);
    set_scheduler(scheduler);
}

// todo: add a scheduler manager to manage multiple schedulers
