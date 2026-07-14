// SPDX-License-Identifier: MPL-2.0

//! This module offers `/proc/uptime` file support, which provides
//! information about the system uptime and idle time.
//!
//! Reference: <https://man7.org/linux/man-pages/man5/proc_uptime.5.html>

use core::time::Duration;

use aster_util::printer::VmPrinter;

use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
    time::{NSEC_PER_SEC, cpu_time_stats::CpuTimeStatsManager},
};

/// Represents the inode at `/proc/uptime`.
pub struct UptimeFileOps;

impl UptimeFileOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference:
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/uptime.c#L45>
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/generic.c#L549-L550>
        ProcFile::new(Self, parent, mkmod!(a+r))
    }

    fn print_uptime(printer: &mut VmPrinter) -> Result<()> {
        let (uptime_secs, uptime_centis) =
            duration_to_seconds_and_centiseconds(aster_time::read_monotonic_time());

        let cpustat = CpuTimeStatsManager::singleton();
        let (idle_secs, idle_centis) = duration_to_seconds_and_centiseconds(
            cpustat.collect_stats_on_all_cpus().idle.as_duration(),
        );

        writeln!(
            printer,
            "{uptime_secs}.{uptime_centis:02} {idle_secs}.{idle_centis:02}"
        )?;
        Ok(())
    }
}

/// Splits a duration into seconds and the centiseconds printed by `/proc/uptime`.
fn duration_to_seconds_and_centiseconds(duration: Duration) -> (u64, u32) {
    let centiseconds = duration.subsec_nanos() / (NSEC_PER_SEC as u32 / 100);
    (duration.as_secs(), centiseconds)
}

impl ProcFileOps for UptimeFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        Self::print_uptime(&mut printer)?;

        Ok(printer.bytes_written())
    }
}

#[cfg(ktest)]
mod tests {
    use core::time::Duration;

    use ostd::prelude::ktest;

    use super::duration_to_seconds_and_centiseconds;

    #[ktest]
    fn uptime_centiseconds_are_truncated() {
        assert_eq!(
            duration_to_seconds_and_centiseconds(Duration::new(42, 0)),
            (42, 0)
        );
        assert_eq!(
            duration_to_seconds_and_centiseconds(Duration::new(42, 9_999_999)),
            (42, 0)
        );
        assert_eq!(
            duration_to_seconds_and_centiseconds(Duration::new(42, 10_000_000)),
            (42, 1)
        );
        assert_eq!(
            duration_to_seconds_and_centiseconds(Duration::new(42, 999_999_999)),
            (42, 99)
        );
    }
}
