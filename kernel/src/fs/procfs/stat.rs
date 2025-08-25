// SPDX-License-Identifier: MPL-2.0

//! This module offers `/proc/stat` file support, which provides
//! information about kernel system statistics.
//!
//! Reference: <https://man7.org/linux/man-pages/man5/proc_stat.5.html>

use alloc::format;
use core::sync::atomic::Ordering;

use aster_softirq::get_softirq_stats;
use ostd::{
    cpu::num_cpus,
    task::get_context_switches,
    trap::{get_interrupt_stats, get_total_interrupts},
};

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::Inode,
    },
    prelude::*,
    process::process_table::TOTAL_FORKS,
    sched::nr_queued_and_running,
    time::{cputime::cpu_stat_manager, SystemTime, START_TIME},
};

pub struct StatFileOps;

impl StatFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self).parent(parent).build().unwrap()
    }

    fn collect_stats() -> String {
        let cpu_count = num_cpus();
        let cpu_manager = cpu_stat_manager();

        // Get global CPU statistics
        let global_stats = cpu_manager.get_global();

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
        for cpu_id in 0..cpu_count {
            let cpu_stats = cpu_manager.get_on_cpu(cpu_id);
            output.push_str(&format!(
                "cpu{} {} {} {} {} {} {} {} {} {} {}\n",
                cpu_id,
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
        let interrupt_stats = get_interrupt_stats();
        let total_interrupts = get_total_interrupts();

        // Build the intr line: total followed by per-IRQ counts
        let mut intr_line = format!("intr {}", total_interrupts);
        for count in interrupt_stats.iter() {
            intr_line.push_str(&format!(" {}", count));
        }
        intr_line.push('\n');
        output.push_str(&intr_line);

        // Context switches
        let context_switches = get_context_switches();
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

        output.push_str(&format!(
            "processes {}\n",
            TOTAL_FORKS.load(Ordering::Relaxed)
        ));

        // Running and blocked processes
        let (_, running_count) = nr_queued_and_running();
        output.push_str(&format!("procs_running {}\n", running_count));

        // TODO: blocked processes
        output.push_str("procs_blocked 0\n");

        // Softirq statistics
        let softirq_stats = get_softirq_stats();
        let total_softirqs: u64 = softirq_stats.iter().sum();
        output.push_str(&format!(
            "softirq {} {} {} {} {} {} {} {} {} {} {}\n",
            total_softirqs,
            softirq_stats[0],                   // TASKLESS_URGENT
            softirq_stats[1],                   // TIMER
            softirq_stats[2],                   // TASKLESS
            softirq_stats[3],                   // NETWORK_TX
            softirq_stats[4],                   // NETWORK_RX
            softirq_stats.get(5).unwrap_or(&0), // Reserved
            softirq_stats.get(6).unwrap_or(&0), // Reserved
            softirq_stats.get(7).unwrap_or(&0), // Reserved
            0u64,                               // Reserved
            0u64                                // Reserved
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
