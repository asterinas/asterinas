// SPDX-License-Identifier: MPL-2.0

use core::fmt::Write;

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::Inode,
    },
    prelude::*,
    Process,
};

/// Represents the inode at `/proc/[pid]/stat`.
/// The fields are the same as the ones in `/proc/[pid]/status`. But the format is different.
/// See https://github.com/torvalds/linux/blob/ce1c54fdff7c4556b08f5b875a331d8952e8b6b7/fs/proc/array.c#L467
/// FIXME: Some fields are not implemented yet.
///
/// Fields:
/// - pid              : Process ID.
/// - comm             : Process name.
/// - state            : Process state (R: running, S: sleeping, Z: zombie).
/// - ppid             : Parent process ID.
/// - pgrp             : Process group ID.
/// - session          : Session ID.
/// - tty_nr           : Terminal associated with the process.
/// - tpgid            : Foreground process group ID.
/// - flags            : Kernel flags determining process behavior.
/// - minflt           : Minor page faults (no I/O needed).
/// - cminflt          : Cumulative minor faults of child processes.
/// - majflt           : Major page faults (I/O required).
/// - cmajflt          : Cumulative major faults of child processes.
/// - utime            : Time spent in user mode (clock ticks).
/// - stime            : Time spent in kernel mode (clock ticks).
/// - cutime           : Child processes' user mode time.
/// - cstime           : Child processes' kernel mode time.
/// - priority         : Process priority or nice value.
/// - nice             : Nice value (-20 to 19; lower is higher priority).
/// - num_threads      : Number of threads.
/// - starttime        : Process start time since boot (clock ticks).
/// - vsize            : Virtual memory size (bytes).
/// - rss              : Resident Set Size (pages in real memory).
/// - rsslim           : Soft memory limit (bytes).
/// - startcode        : Start address of executable code.
/// - endcode          : End address of executable code.
/// - startstack       : Bottom address of process stack.
/// - kstkesp          : Current stack pointer (ESP).
/// - kstkeip          : Current instruction pointer (EIP).
/// - signal           : Bitmap of pending signals.
/// - blocked          : Bitmap of blocked signals.
/// - sigignore        : Bitmap of ignored signals.
/// - sigcatch         : Bitmap of caught signals.
/// - wchan            : Address where the process is waiting.
/// - nswap            : Number of pages swapped (deprecated).
/// - cnswap           : Cumulative swapped pages of children.
/// - exit_signal      : Signal sent to parent on termination.
/// - processor        : Last CPU the process executed on.
/// - rt_priority      : Real-time scheduling priority (1-99, 0 otherwise).
/// - policy           : Scheduling policy (e.g., SCHED_NORMAL, SCHED_FIFO).
/// - delayacct_blkio_ticks : Block I/O delays (clock ticks).
/// - guest_time       : Time spent as a guest in virtual CPU.
/// - cguest_time      : Guest time of child processes.
/// - start_data       : Start address of initialized/uninitialized data.
/// - end_data         : End address of initialized/uninitialized data.
/// - start_brk        : Address above which the heap expands.
/// - arg_start        : Start address of command-line arguments.
/// - arg_end          : End address of command-line arguments.
/// - env_start        : Start address of environment variables.
/// - env_end          : End address of environment variables.
/// - exit_code        : Process exit code as returned by waitpid(2).
pub struct StatFileOps(Arc<Process>);

impl StatFileOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self(process_ref))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl FileOps for StatFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let process = &self.0;

        let pid = process.pid();
        let comm = process.executable_path();
        let state = if process.status().is_zombie() {
            'Z'
        } else {
            'R'
        };
        let ppid = process.parent().pid();
        let pgrp = process.pgid();

        let mut stat_output = String::new();
        writeln!(
            stat_output,
            "{} ({}) {} {} {}",
            pid, comm, state, ppid, pgrp
        )
        .unwrap();
        Ok(stat_output.into_bytes())
    }
}
