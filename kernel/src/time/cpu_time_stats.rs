// SPDX-License-Identifier: MPL-2.0
use aster_util::per_cpu_counter::PerCpuCounter;
use ostd::{
    cpu::{CpuId, PrivilegeLevel},
    irq::InterruptLevel,
    timer::Jiffies,
};
use spin::Once;

use crate::{sched::SchedPolicy, thread::Thread};

/// Represents CPU usage statistics for a system.
///
/// This structure contains various counters that track different types of CPU time.
/// All values are measured in jiffies (clock ticks).
///
/// TODO: Implement proper accounting for CPU time
#[derive(Debug, Clone, Copy)]
pub struct CpuTimeStats {
    /// Time spent in user mode.
    pub user: Jiffies,
    /// Time spent in user mode with low priority (nice).
    pub nice: Jiffies,
    /// Time spent in system/kernel mode.
    pub system: Jiffies,
    /// Time spent in the idle task.
    pub idle: Jiffies,
    /// Time spent waiting for I/O to complete.
    /// TODO: track this statistic.
    pub iowait: Jiffies,
    /// Time spent servicing hardware interrupts.
    pub irq: Jiffies,
    /// Time spent servicing software interrupts.
    pub softirq: Jiffies,
    /// Time stolen by other operating systems running in a virtualized environment.
    /// TODO: track this statistic.
    pub steal: Jiffies,
    /// Time spent running a virtual CPU for guest operating systems.
    /// TODO: track this statistic.
    pub guest: Jiffies,
    /// Time spent running a low priority virtual CPU for guest operating systems.
    /// TODO: track this statistic.
    pub guest_nice: Jiffies,
}

pub struct CpuTimeStatsManager {
    user: PerCpuCounter,
    nice: PerCpuCounter,
    system: PerCpuCounter,
    idle: PerCpuCounter,
    iowait: PerCpuCounter,
    irq: PerCpuCounter,
    softirq: PerCpuCounter,
    steal: PerCpuCounter,
    guest: PerCpuCounter,
    guest_nice: PerCpuCounter,
}

static SINGLETON: Once<CpuTimeStatsManager> = Once::new();

impl CpuTimeStatsManager {
    /// Returns a reference to the singleton instance of `CpuTimeStatsManager`.
    pub fn singleton() -> &'static CpuTimeStatsManager {
        // It's fine to `unwrap` because `SINGLETON` must have been initialized in `init`.
        SINGLETON.get().unwrap()
    }

    /// Collects the time statistics on the specific CPU.  
    pub fn collect_stats_on_cpu(&self, cpu: CpuId) -> CpuTimeStats {
        CpuTimeStats {
            user: Jiffies::new(self.user.get_on_cpu(cpu) as u64),
            nice: Jiffies::new(self.nice.get_on_cpu(cpu) as u64),
            system: Jiffies::new(self.system.get_on_cpu(cpu) as u64),
            idle: Jiffies::new(self.idle.get_on_cpu(cpu) as u64),
            iowait: Jiffies::new(self.iowait.get_on_cpu(cpu) as u64),
            irq: Jiffies::new(self.irq.get_on_cpu(cpu) as u64),
            softirq: Jiffies::new(self.softirq.get_on_cpu(cpu) as u64),
            steal: Jiffies::new(self.steal.get_on_cpu(cpu) as u64),
            guest: Jiffies::new(self.guest.get_on_cpu(cpu) as u64),
            guest_nice: Jiffies::new(self.guest_nice.get_on_cpu(cpu) as u64),
        }
    }

    /// Collects the time statistics across all CPUs.
    pub fn collect_stats_on_all_cpus(&self) -> CpuTimeStats {
        CpuTimeStats {
            user: Jiffies::new(self.user.sum_all_cpus() as u64),
            nice: Jiffies::new(self.nice.sum_all_cpus() as u64),
            system: Jiffies::new(self.system.sum_all_cpus() as u64),
            idle: Jiffies::new(self.idle.sum_all_cpus() as u64),
            iowait: Jiffies::new(self.iowait.sum_all_cpus() as u64),
            irq: Jiffies::new(self.irq.sum_all_cpus() as u64),
            softirq: Jiffies::new(self.softirq.sum_all_cpus() as u64),
            steal: Jiffies::new(self.steal.sum_all_cpus() as u64),
            guest: Jiffies::new(self.guest.sum_all_cpus() as u64),
            guest_nice: Jiffies::new(self.guest_nice.sum_all_cpus() as u64),
        }
    }

    fn inc_user_time(&self, cpu: CpuId) {
        self.user.add_on_cpu(cpu, 1);
    }

    fn inc_system_time(&self, cpu: CpuId) {
        self.system.add_on_cpu(cpu, 1);
    }

    fn inc_idle_time(&self, cpu: CpuId) {
        self.idle.add_on_cpu(cpu, 1);
    }

    fn new() -> Self {
        Self {
            user: PerCpuCounter::new(),
            nice: PerCpuCounter::new(),
            system: PerCpuCounter::new(),
            idle: PerCpuCounter::new(),
            iowait: PerCpuCounter::new(),
            irq: PerCpuCounter::new(),
            softirq: PerCpuCounter::new(),
            steal: PerCpuCounter::new(),
            guest: PerCpuCounter::new(),
            guest_nice: PerCpuCounter::new(),
        }
    }
}

fn update_cpu_statistics() {
    let manager = CpuTimeStatsManager::singleton();

    // No races because we are in IRQs.
    let cpu_id = CpuId::current_racy();

    match InterruptLevel::current() {
        // The kernel code is interrupted.
        InterruptLevel::L1(PrivilegeLevel::Kernel) => {
            if is_idle() {
                // Idle time is not counted towards CPU usage.
                manager.inc_idle_time(cpu_id)
            } else {
                // Non-idle time is counted as kernel time.
                manager.inc_system_time(cpu_id);
            }
        }
        // The user code is interrupted.
        InterruptLevel::L1(PrivilegeLevel::User) => manager.inc_user_time(cpu_id),
        // The interrupt code is interrupted.
        InterruptLevel::L2 => manager.inc_system_time(cpu_id),

        // We're handling timer interrupts, so this is unreachable.
        InterruptLevel::L0 => unreachable!("interrupts must not run in the task context"),
    }
}

fn is_idle() -> bool {
    if let Some(current_thread) = Thread::current() {
        current_thread.sched_attr().policy() == SchedPolicy::Idle
    } else {
        false
    }
}

pub fn init() {
    SINGLETON.call_once(CpuTimeStatsManager::new);
}

pub fn init_on_each_cpu() {
    ostd::timer::register_callback_on_cpu(update_cpu_statistics);
}
