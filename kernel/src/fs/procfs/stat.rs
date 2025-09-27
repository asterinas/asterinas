// SPDX-License-Identifier: MPL-2.0

//! This module offers `/proc/stat` file support, which provides
//! information about kernel system statistics.
//!
//! Reference: <https://man7.org/linux/man-pages/man5/proc_stat.5.html>

use core::fmt::Write;

use aster_softirq::{
    iter_irq_counts_across_all_cpus, iter_softirq_counts_across_all_cpus, softirq_id::*,
};

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{mkmod, Inode},
    },
    prelude::*,
    process::collect_process_creation_count,
    sched::nr_queued_and_running,
    thread::collect_context_switch_count,
    time::{cpu_time_stats::CpuTimeStatsManager, SystemTime, START_TIME},
};

/// Represents the inode at `/proc/stat`.  
pub struct StatFileOps;

impl StatFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference:
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/stat.c#L213>
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/generic.c#L549-L550>
        ProcFileBuilder::new(Self, mkmod!(a+r))
            .parent(parent)
            .build()
            .unwrap()
    }

    fn collect_stats() -> String {
        let mut stat_output = String::new();

        let stats_manager = CpuTimeStatsManager::singleton();

        // Global CPU statistics:
        // cpu <user> <nice> <system> <idle> <iowait> <irq> <softirq> <steal> <guest> <guest_nice>
        let global_stats = stats_manager.collect_stats_on_all_cpus();
        writeln!(
            stat_output,
            "cpu {} {} {} {} {} {} {} {} {} {}",
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
        )
        .unwrap();

        // Per-CPU statistics:
        for cpu_id in ostd::cpu::all_cpus() {
            let cpu_stats = stats_manager.collect_stats_on_cpu(cpu_id);
            writeln!(
                stat_output,
                "cpu{} {} {} {} {} {} {} {} {} {} {}",
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
            )
            .unwrap();
        }

        // IRQ statistics: the total count followed by per-IRQ counts
        let irq_stats = iter_irq_counts_across_all_cpus();
        let mut total_irqs = 0usize;
        let mut irq_counts = Vec::new();
        for count in irq_stats {
            total_irqs += count;
            irq_counts.push(count);
        }
        write!(stat_output, "intr {}", total_irqs).unwrap();
        for count in irq_counts {
            write!(stat_output, " {}", count).unwrap();
        }

        writeln!(stat_output).unwrap();

        // Context switch count
        let context_switches: usize = collect_context_switch_count();
        writeln!(stat_output, "ctxt {}", context_switches).unwrap();

        // Boot time (seconds since UNIX epoch)
        if let Some(start_time) = START_TIME.get() {
            let boot_time = start_time
                .duration_since(&SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            writeln!(stat_output, "btime {}", boot_time).unwrap();
        } else {
            writeln!(stat_output, "btime {}", 0).unwrap();
        }

        // Process count (number of created processes since boot)
        writeln!(
            stat_output,
            "processes {}",
            collect_process_creation_count()
        )
        .unwrap();

        // Running and blocked processes
        let (_, running_count) = nr_queued_and_running();
        writeln!(stat_output, "procs_running {}", running_count).unwrap();

        // TODO: Blocked processes
        writeln!(stat_output, "procs_blocked {}", 0).unwrap();

        // Softirq statistics
        let softirq_stats = iter_softirq_counts_across_all_cpus();
        let softirq_stats: Vec<usize> = softirq_stats.collect();
        let total_softirqs: usize = softirq_stats.iter().sum();

        // We only have 5 defined softirq types; the rest are reserved.
        // Fill in zeros for the reserved types to match the expected output format.
        writeln!(
            stat_output,
            "softirq {} {} {} {} {} {} {} {} {} {} {}",
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
            0usize,                                             // Reserved
        )
        .unwrap();

        stat_output
    }
}

impl FileOps for StatFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let output = Self::collect_stats();
        Ok(output.into_bytes())
    }
}
