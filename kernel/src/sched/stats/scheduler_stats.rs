// SPDX-License-Identifier: MPL-2.0

use ostd::timer;
use spin::Once;

use super::loadavg;

/// The global scheduler statistic singleton
static SCHEDULER_STATS: Once<&'static dyn SchedulerStats> = Once::new();

/// Set the global scheduler statistics singleton.
///
/// This function should be called once to set the scheduler statistics system.
/// It is used to get running stats from the scheduler and to periodically
/// calculate the system load average.
pub fn set_stats_from_scheduler(scheduler: &'static dyn SchedulerStats) {
    SCHEDULER_STATS.call_once(|| scheduler);

    // Register a callback to update the load average periodically
    timer::register_callback(|| {
        loadavg::update_loadavg(|| nr_queued_and_running().0);
    });
}

/// The trait for the scheduler statistics.
pub trait SchedulerStats: Sync + Send {
    /// Returns a tuple with the number of tasks in the runqueues and the number of running tasks.
    ///
    /// We decided to return a tuple instead of having two separate functions to
    /// avoid the overhead of disabling the preemption twice to inspect the scheduler.
    fn nr_queued_and_running(&self) -> (u32, u32);
}

/// Get the amount of tasks in the runqueues and the amount of running tasks.
pub fn nr_queued_and_running() -> (u32, u32) {
    SCHEDULER_STATS.get().unwrap().nr_queued_and_running()
}
