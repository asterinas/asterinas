// SPDX-License-Identifier: MPL-2.0

use aster_util::printer::VmPrinter;

use super::TidDirOps;
use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::{Inode, mkmod},
    },
    prelude::*,
    process::posix_thread::{AsPosixThread, SleepingState},
    vm::vmar::RssType,
};

/// Represents the inode at `/proc/[pid]/task/[tid]/status` (and also `/proc/[pid]/status`).
/// See <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/array.c#L148>.
/// FIXME: Some fields are not implemented yet.
///
/// Fields:
/// - Name:   The name of the process.
/// - State:  The current state of the process (e.g., R for running, S for sleeping).
/// - Tgid:   The Thread Group ID, which is the same as the process ID for the main thread.
/// - Pid:    The process ID.
/// - PPid:   The parent process ID.
/// - TracerPid: The PID of the process tracing this process, or 0 if not being traced.
/// - Uid:    Real, effective, saved set, and filesystem UIDs.
/// - Gid:    Real, effective, saved set, and filesystem GIDs.
/// - FDSize: The number of file descriptor slots currently allocated.
/// - Groups: Supplementary group IDs.
/// - VmPeak: Peak virtual memory size.
/// - VmSize: Current virtual memory size.
/// - VmLck:  Locked memory size.
/// - VmPin:  Pinned memory size.
/// - VmHWM:  Peak resident set size ("high water mark").
/// - VmRSS:  Resident set size.
/// - VmData: Size of data segment.
/// - VmStk:  Size of stack segment.
/// - VmExe:  Size of text segment.
/// - VmLib:  Shared library code size.
/// - VmPTE:  Page table entries size.
/// - VmSwap: Swapped-out virtual memory size by anonymous private pages.
/// - Threads: Number of threads in this process.
/// - SigQ:   Current signal queue size and limit.
/// - SigPnd: Threads pending signals.
/// - ShdPnd: Shared pending signals.
/// - SigBlk: Blocked signals.
/// - SigIgn: Ignored signals.
/// - SigCgt: Caught signals.
/// - CapInh: Inheritable capabilities.
/// - CapPrm: Permitted capabilities.
/// - CapEff: Effective capabilities.
/// - CapBnd: Bounding set.
/// - CapAmb: Ambient capabilities.
/// - Seccomp: Seccomp mode.
/// - Cpus_allowed: CPUs allowed for this process.
/// - Cpus_allowed_list: List of CPUs allowed for this process.
/// - Mems_allowed: Memory nodes allowed for this process.
/// - Mems_allowed_list: List of memory nodes allowed for this process.
/// - voluntary_ctxt_switches: Number of voluntary context switches.
/// - nonvoluntary_ctxt_switches: Number of nonvoluntary context switches.
pub struct StatusFileOps(TidDirOps);

impl StatusFileOps {
    pub fn new_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3326>
        ProcFileBuilder::new(Self(dir.clone()), mkmod!(a+r))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for StatusFileOps {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let mut printer = VmPrinter::new_skip(writer, offset);

        let process = self.0.process_ref.as_ref();
        let thread = self.0.thread();
        let posix_thread = thread.as_posix_thread().unwrap();

        // According to the Linux implementation, a process's `/proc/<pid>/status`
        // is exactly the same as its main thread's `/proc/<pid>/task/<pid>/status`.
        //
        // Reference:
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3326>
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3675>

        writeln!(
            printer,
            "Name:\t{}",
            posix_thread.thread_name().lock().name().to_string_lossy()
        )?;

        let state = if thread.is_exited() {
            "Z (zombie)"
        } else {
            match posix_thread.sleeping_state() {
                SleepingState::Running => "R (running)",
                SleepingState::Interruptible => "S (sleeping)",
                SleepingState::Uninterruptible => "D (disk sleep)",
                SleepingState::StopBySignal => "T (stopped)",
                SleepingState::StopByPtrace => "t (tracing stop)",
            }
        };
        writeln!(printer, "State:\t{}", state)?;

        writeln!(printer, "Tgid:\t{}", process.pid())?;
        writeln!(printer, "Pid:\t{}", posix_thread.tid())?;
        writeln!(printer, "PPid:\t{}", process.parent().pid())?;
        writeln!(printer, "TracerPid:\t{}", 0)?;

        let credentials = posix_thread.credentials();
        writeln!(
            printer,
            "Uid:\t{}\t{}\t{}\t{}",
            u32::from(credentials.ruid()),
            u32::from(credentials.euid()),
            u32::from(credentials.suid()),
            u32::from(credentials.fsuid()),
        )?;
        writeln!(
            printer,
            "Gid:\t{}\t{}\t{}\t{}",
            u32::from(credentials.rgid()),
            u32::from(credentials.egid()),
            u32::from(credentials.sgid()),
            u32::from(credentials.fsgid()),
        )?;

        writeln!(
            printer,
            "FDSize:\t{}",
            posix_thread
                .file_table()
                .lock()
                .as_ref()
                .map(|file_table| file_table.read().len())
                .unwrap_or(0)
        )?;

        if let Some(vmar_ref) = process.lock_vmar().as_ref() {
            let vsize = vmar_ref.get_mappings_total_size();
            let anon = vmar_ref.get_rss_counter(RssType::RSS_ANONPAGES) * (PAGE_SIZE / 1024);
            let file = vmar_ref.get_rss_counter(RssType::RSS_FILEPAGES) * (PAGE_SIZE / 1024);
            let rss = anon + file;
            writeln!(
                printer,
                "VmSize:\t{} kB\nVmRSS:\t{} kB\nRssAnon:\t{} kB\nRssFile:\t{} kB",
                vsize, rss, anon, file
            )?;
        }

        if process.pid() == posix_thread.tid() {
            writeln!(
                printer,
                "Threads:\t{}",
                process.tasks().lock().as_slice().len()
            )?;
        }

        Ok(printer.bytes_written())
    }
}
