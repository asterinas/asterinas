// SPDX-License-Identifier: MPL-2.0

use aster_util::per_cpu_counter::PerCpuCounter;
use spin::Once;

pub(super) static CONTEXT_SWITCH_COUNTER: Once<PerCpuCounter> = Once::new();

/// Counts the number of context switches ever happened across all CPUs.
pub fn collect_context_switch_count() -> usize {
    CONTEXT_SWITCH_COUNTER.get().unwrap().sum_all_cpus()
}
