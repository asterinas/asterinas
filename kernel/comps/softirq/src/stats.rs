// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;

use aster_util::per_cpu_counter::PerCpuCounter;
use ostd::cpu::CpuId;
use spin::Once;

use super::SoftIrqLine;

/// The maximum number of IRQ lines we track (256 should have covered most common hardware).
pub(super) const NR_IRQ_LINES: usize = 256;

/// Global interrupt counter.
pub(super) static IRQ_COUNTERS: Once<[PerCpuCounter; NR_IRQ_LINES]> = Once::new();

/// Iterates all IRQ lines for the number of executions across all CPUs.  
pub fn iter_irq_counts_across_all_cpus() -> impl Iterator<Item = usize> {
    let irq_counters = IRQ_COUNTERS.get().unwrap();
    irq_counters.iter().map(|counter| counter.sum_all_cpus())
}

pub(super) fn process_statistic(irq_num: usize) {
    if irq_num < NR_IRQ_LINES {
        IRQ_COUNTERS.get().unwrap()[irq_num].add_on_cpu(CpuId::current_racy(), 1);
    }
}

/// Iterates all softirq lines for the number of executions across all CPUs.
pub fn iter_softirq_counts_across_all_cpus() -> impl Iterator<Item = usize> {
    let softirq_counters: Vec<usize> = (0..SoftIrqLine::NR_LINES)
        .map(|i| {
            let soft_irq_line = SoftIrqLine::get(i);
            if soft_irq_line.is_enabled() {
                soft_irq_line.counter.get().unwrap().sum_all_cpus()
            } else {
                0
            }
        })
        .collect::<Vec<usize>>();
    softirq_counters.into_iter()
}

/// Iterates the softirq counters for a specific CPU.
pub fn iter_softirq_counts_on_cpu(cpuid: CpuId) -> impl Iterator<Item = usize> {
    let softirq_counters = (0..SoftIrqLine::NR_LINES)
        .map(|i| {
            let soft_irq_line = SoftIrqLine::get(i);
            if soft_irq_line.is_enabled() {
                soft_irq_line.counter.get().unwrap().get_on_cpu(cpuid)
            } else {
                0
            }
        })
        .collect::<Vec<usize>>();
    softirq_counters.into_iter()
}
