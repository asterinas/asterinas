// SPDX-License-Identifier: MPL-2.0

pub mod nice;
mod scheduler;

use alloc::boxed::Box;

use aster_frame::task::set_scheduler;
use scheduler::fifo_with_rt_preempt::PreemptiveFIFOScheduler;

pub fn init() {
    let sched = Box::new(PreemptiveFIFOScheduler::new());
    let sched = Box::<PreemptiveFIFOScheduler>::leak(sched);
    set_scheduler(sched);
}
