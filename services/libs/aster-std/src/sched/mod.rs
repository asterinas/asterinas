mod scheduler;

use crate::prelude::*;
use aster_frame::task::set_scheduler;
use scheduler::rt_round_robin::PreemptiveRRScheduler;

// There may be multiple scheduling policies in the system,
// and subsequent schedulers can be placed under this module.
pub fn init() {
    let sched = Box::new(PreemptiveRRScheduler::new());
    let sched = Box::<PreemptiveRRScheduler>::leak(sched);
    set_scheduler(sched);
}

// todo: add a scheduler manager to manage multiple schedulers
