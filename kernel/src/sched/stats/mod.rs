// SPDX-License-Identifier: MPL-2.0

pub mod loadavg;
mod scheduler_stats;

pub use scheduler_stats::{nr_queued_and_running, set_stats_from_scheduler, SchedulerStats};
