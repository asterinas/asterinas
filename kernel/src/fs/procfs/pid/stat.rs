// SPDX-License-Identifier: MPL-2.0

use core::{fmt::Write, sync::atomic::Ordering};

use crate::{
    fs::{
        procfs::template::{FileOps, ProcFileBuilder},
        utils::Inode,
    },
    prelude::*,
    process::posix_thread::AsPosixThread,
    thread::Thread,
    vm::vmar::RssType,
    Process,
};

/// Represents the inode at either `/proc/[pid]/stat` or `/proc/[pid]/task/[tid]/stat`.
///
/// The fields are the same as the ones in `/proc/[pid]/status`, but the format is different.
/// See <https://github.com/torvalds/linux/blob/ce1c54fdff7c4556b08f5b875a331d8952e8b6b7/fs/proc/array.c#L467>.
///
/// FIXME: Some fields are not implemented or contain placeholders yet.
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
pub struct StatFileOps {
    process_ref: Arc<Process>,
    thread_ref: Arc<Thread>,
    /// If `is_pid_stat` is true, this file corresponds to a process-level `/proc/[pid]/stat`.
    /// Otherwise, this file corresponds to the thread-level `/proc/[pid]/task/[tid]/stat`.
    is_pid_stat: bool,
}

impl StatFileOps {
    pub fn new_inode(
        process_ref: Arc<Process>,
        thread_ref: Arc<Thread>,
        is_pid_stat: bool,
        parent: Weak<dyn Inode>,
    ) -> Arc<dyn Inode> {
        ProcFileBuilder::new(Self {
            process_ref,
            thread_ref,
            is_pid_stat,
        })
        .parent(parent)
        .build()
        .unwrap()
    }
}

impl FileOps for StatFileOps {
    fn data(&self) -> Result<Vec<u8>> {
        let process = &self.process_ref;
        let thread = &self.thread_ref;
        let posix_thread = thread.as_posix_thread().unwrap();

        // According to the Linux implementation, a process's `/proc/<pid>/stat` should be
        // almost identical to its main thread's `/proc/<pid>/task/<pid>/stat`, except for
        // fields `exit_code`, `wchan`, `min_flt`, `maj_flt`, `gtime`, `utime`, and `stime`.
        //
        // Reference: <https://github.com/torvalds/linux/blob/0ff41df1cb268fc69e703a08a57ee14ae967d0ca/fs/proc/array.c#L467-L681>

        let pid = posix_thread.tid();
        let comm = posix_thread
            .thread_name()
            .lock()
            .as_ref()
            .and_then(|name| name.as_string())
            .unwrap_or_else(|| process.executable_path());
        let state = if thread.is_exited() { 'Z' } else { 'R' };
        let ppid = process.parent().pid();
        let pgrp = process.pgid();
        let session = process.sid();

        let (tty_nr, tpgid) = if let Some(terminal) = process.terminal() {
            (
                terminal.id().as_encoded_u64(),
                terminal
                    .job_control()
                    .foreground()
                    .map(|pgrp| pgrp.pgid() as i64)
                    .unwrap_or(-1),
            )
        } else {
            (0, -1)
        };

        let flags = 0;
        let min_flt = 0;
        let cmin_flt = 0;
        let maj_flt = 0;
        let cmaj_flt = 0;

        let (utime, stime) = {
            let prof_clock = if self.is_pid_stat {
                process.prof_clock()
            } else {
                posix_thread.prof_clock()
            };
            (
                prof_clock.user_clock().read_jiffies().as_u64(),
                prof_clock.kernel_clock().read_jiffies().as_u64(),
            )
        };

        let cutime = 0;
        let cstime = 0;
        let priority = 0;
        let nice = process.nice().load(Ordering::Relaxed).value().get();
        let num_threads = process.tasks().lock().as_slice().len();
        let itrealvalue = 0;
        let starttime = 0;

        let (vsize, rss) = if let Some(vmar_ref) = process.lock_root_vmar().as_ref() {
            let vsize = vmar_ref.get_mappings_total_size();
            let anon = vmar_ref.get_rss_counter(RssType::RSS_ANONPAGES);
            let file = vmar_ref.get_rss_counter(RssType::RSS_FILEPAGES);
            let rss = anon + file;
            (vsize, rss)
        } else {
            (0, 0)
        };

        let mut stat_output = String::new();
        writeln!(
            stat_output,
            "{} ({}) {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {}",
            pid,
            comm,
            state,
            ppid,
            pgrp,
            session,
            tty_nr,
            tpgid,
            flags,
            min_flt,
            cmin_flt,
            maj_flt,
            cmaj_flt,
            utime,
            stime,
            cutime,
            cstime,
            priority,
            nice,
            num_threads,
            itrealvalue,
            starttime,
            vsize,
            rss
        )
        .unwrap();
        Ok(stat_output.into_bytes())
    }
}
