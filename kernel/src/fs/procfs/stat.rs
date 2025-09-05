// SPDX-License-Identifier: MPL-2.0

//! This module offers `/proc/stat` file support, which provides
//! information about kernel system statistics.
//!
//! Reference: <https://man7.org/linux/man-pages/man5/proc_stat.5.html>

use alloc::format;

use aster_softirq::{
    collect_per_irq_counts_across_all_cpus, collect_per_softirq_counts_across_all_cpus,
    softirq_id::*,
};

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::Inode,
    },
    prelude::*,
    process::forks_count,
    sched::nr_queued_and_running,
    thread::context_switch_count,
    time::{cpu_time_stats::CpuTimeStatsManager, SystemTime, START_TIME},
};

pub struct StatFileOps;

impl StatFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self).parent(parent).build().unwrap()
    }

    fn collect_stats() -> String {
        let cpu_manager = CpuTimeStatsManager::singleton();

        // Get global CPU statistics
        let global_stats = cpu_manager.collect_stats_on_all_cpus();

        let mut output = String::new();

        // Global CPU line: cpu <user> <nice> <system> <idle> <iowait> <irq> <softirq> <steal> <guest> <guest_nice>
        output.push_str(&format!(
            "cpu {} {} {} {} {} {} {} {} {} {}\n",
            global_stats.user.as_u64(),
            global_stats.nice.as_u64(),
            global_stats.system.as_u64(),
            global_stats.idle.as_u64(),
            global_stats.iowait.as_u64(),
            global_stats.irq.as_u64(),
            global_stats.softirq.as_u64(),
            global_stats.steal.as_u64(),
            global_stats.guest.as_u64(),
            global_stats.guest_nice.as_u64()
        ));

        // Per-CPU lines
        for cpu_id in ostd::cpu::all_cpus() {
            let cpu_stats = cpu_manager.collect_stats_on_cpu(cpu_id);
            output.push_str(&format!(
                "cpu{} {} {} {} {} {} {} {} {} {} {}\n",
                cpu_id.as_usize(),
                cpu_stats.user.as_u64(),
                cpu_stats.nice.as_u64(),
                cpu_stats.system.as_u64(),
                cpu_stats.idle.as_u64(),
                cpu_stats.iowait.as_u64(),
                cpu_stats.irq.as_u64(),
                cpu_stats.softirq.as_u64(),
                cpu_stats.steal.as_u64(),
                cpu_stats.guest.as_u64(),
                cpu_stats.guest_nice.as_u64()
            ));
        }

        // Interrupt count with per-IRQ breakdown
        let irq_stats = collect_per_irq_counts_across_all_cpus();
        let total_irqs: usize = irq_stats.iter().sum();
        // Build the irq line: total followed by per-IRQ counts
        let mut irq_line = format!("irq {}", total_irqs);
        for count in irq_stats.iter() {
            irq_line.push_str(&format!(" {}", count));
        }
        irq_line.push('\n');
        output.push_str(&irq_line);

        // Context switch count
        let context_switches: usize = context_switch_count();
        output.push_str(&format!("ctxt {}\n", context_switches));

        // Boot time (seconds since UNIX epoch)
        if let Some(start_time) = START_TIME.get() {
            let boot_time = start_time
                .duration_since(&SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            output.push_str(&format!("btime {}\n", boot_time));
        } else {
            output.push_str("btime 0\n");
        }

        output.push_str(&format!("processes {}\n", forks_count()));

        // Running and blocked processes
        let (_, running_count) = nr_queued_and_running();
        output.push_str(&format!("procs_running {}\n", running_count));

        // TODO: Blocked processes
        output.push_str("procs_blocked 0\n");

        // Softirq statistics
        let softirq_stats = collect_per_softirq_counts_across_all_cpus();
        let total_softirqs: usize = softirq_stats.iter().sum();

        output.push_str(&format!(
            "softirq {} {} {} {} {} {} {} {} {} {} {}\n",
            total_softirqs,
            softirq_stats[TASKLESS_URGENT_SOFTIRQ_ID as usize], // TASKLESS_URGENT
            softirq_stats[TIMER_SOFTIRQ_ID as usize],           // TIMER
            softirq_stats[TASKLESS_SOFTIRQ_ID as usize],        // TASKLESS
            softirq_stats[NETWORK_TX_SOFTIRQ_ID as usize],      // NETWORK_TX
            softirq_stats[NETWORK_RX_SOFTIRQ_ID as usize],      // NETWORK_RX
            0usize,                                             // Reserved
            0usize,                                             // Reserved
            0usize,                                             // Reserved
            0usize,                                             // Reserved
            0usize                                              // Reserved
        ));

        output
    }
}

impl FileOps for StatFileOps {
    /// Retrieve the data for `/proc/stat`.
    fn data(&self) -> Result<Vec<u8>> {
        // Implementation to gather and format the statistics
        let output = Self::collect_stats();
        Ok(output.into_bytes())
    }
}
