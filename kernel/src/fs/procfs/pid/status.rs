// SPDX-License-Identifier: MPL-2.0

use core::fmt::Write;

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::Inode,
    },
    prelude::*,
    process::posix_thread::AsPosixThread,
    Process,
};

/// Represents the inode at `/proc/[pid]/status`.
/// See https://github.com/torvalds/linux/blob/ce1c54fdff7c4556b08f5b875a331d8952e8b6b7/fs/proc/array.c#L148
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
pub struct StatusFileOps(Arc<Process>);

impl StatusFileOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self(process_ref))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for StatusFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let process = &self.0;
        let main_thread = process.main_thread().unwrap();
        let file_table = main_thread.as_posix_thread().unwrap().file_table();

        let mut status_output = String::new();
        writeln!(status_output, "Name:\t{}", process.executable_path()).unwrap();
        writeln!(status_output, "Tgid:\t{}", process.pid()).unwrap();
        writeln!(status_output, "Pid:\t{}", process.pid()).unwrap();
        writeln!(status_output, "PPid:\t{}", process.parent().pid()).unwrap();
        writeln!(status_output, "TracerPid:\t{}", process.parent().pid()).unwrap(); // Assuming TracerPid is the same as PPid
        writeln!(status_output, "FDSize:\t{}", file_table.lock().len()).unwrap();
        writeln!(status_output, "Threads:\t{}", process.tasks().lock().len()).unwrap();
        Ok(status_output.into_bytes())
    }
}
