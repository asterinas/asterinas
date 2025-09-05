use aster_util::per_cpu_counter::PerCpuCounter;
use ostd::cpu::CpuId;
use spin::Once;

use super::SoftIrqLine;

/// The maximum number of IRQ lines we track (256 should have covered most common hardware).
pub const MAX_IRQ_LINES: usize = 256;

/// Global interrupt counter.
pub(super) static INTERRUPT_COUNTERS: Once<[PerCpuCounter; MAX_IRQ_LINES]> = Once::new();

/// Returns the interrupt counters for all IRQ lines.
pub fn collect_per_irq_counts_across_all_cpus() -> [usize; MAX_IRQ_LINES] {
    core::array::from_fn(|i| INTERRUPT_COUNTERS.get().unwrap()[i].sum_all_cpus())
}

pub(super) fn process_statistic(irq_number: usize, irq_count: usize) {
    if (irq_count as isize) < 0 {
        // This should never happen.
        return;
    }
    if irq_number < MAX_IRQ_LINES {
        INTERRUPT_COUNTERS.get().unwrap()[irq_number]
            .add_on_cpu(CpuId::current_racy(), irq_count as isize);
    }
}

/// A per-CPU counter for softirq execution.
pub(super) static SOFTIRQ_COUNTERS: Once<[PerCpuCounter; SoftIrqLine::NR_LINES as usize]> =
    Once::new();

/// Returns the execution count for all softirq lines.
pub fn collect_per_softirq_counts_across_all_cpus() -> [usize; SoftIrqLine::NR_LINES as usize] {
    core::array::from_fn(|i| SOFTIRQ_COUNTERS.get().unwrap()[i].sum_all_cpus())
}

/// Returns the softirq counters for a specific CPU.
pub fn collect_per_softirq_counts_on_cpu(cpuid: CpuId) -> [usize; SoftIrqLine::NR_LINES as usize] {
    core::array::from_fn(|i| SOFTIRQ_COUNTERS.get().unwrap()[i].get_on_cpu(cpuid))
}
