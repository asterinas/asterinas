// SPDX-License-Identifier: MPL-2.0

pub(crate) mod loadavg;
mod scheduler_stats;

pub(crate) use scheduler_stats::{SchedulerStats, nr_queued_and_running, set_stats_from_scheduler};
