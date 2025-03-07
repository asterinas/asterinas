// SPDX-License-Identifier: MPL-2.0

mod nice;
mod sched_class;
mod stats;

use core::sync::atomic::{AtomicBool, Ordering};

use ostd::{cpu::PinCurrentCpu, cpu_local};

pub use self::{
    nice::{AtomicNice, Nice},
    sched_class::{init, RealTimePolicy, RealTimePriority, SchedAttr, SchedPolicy},
    stats::{loadavg, nr_queued_and_running},
};

cpu_local! {
    pub static IS_IDLE: AtomicBool = AtomicBool::new(false);
}

/// The idle loop that should be called in an idle thread.
///
/// This function will halt the CPU until:
///  1. there is a thread to run, or
///  2. `cond` returns `true`.
pub(crate) fn idle_until(mut cond: impl FnMut() -> bool) {
    let cpu_id = {
        let preempt_guard = ostd::task::disable_preempt();
        preempt_guard.current_cpu()
    };

    while !cond() {
        crate::thread::Thread::yield_now();
        // If the scheduler picks this thread again, it means that there is no
        // meaningful thread to run.
        IS_IDLE.get_on_cpu(cpu_id).store(true, Ordering::Relaxed);

        ostd::cpu::sleep_for_interrupt();

        IS_IDLE.get_on_cpu(cpu_id).store(false, Ordering::Relaxed);
    }
}
