// SPDX-License-Identifier: MPL-2.0

use core::{sync::atomic::Ordering, time::Duration};

use aster_util::printer::VmPrinter;
use ostd::timer::TIMER_FREQ;

use super::{super::PidDirOps, TidDirOps};
use crate::{
    fs::{
        file::mkmod,
        procfs::template::{ProcFile, ProcFileOps},
        vfs::inode::Inode,
    },
    prelude::*,
    process::{
        Process, ResourceType,
        posix_thread::{AsPosixThread, SleepingState},
        signal::{HandlePendingSignal, sig_action::SigAction, sig_mask::SigMask},
    },
    sched::{LinuxSchedPolicy, RealTimePriority, SchedPolicy},
    thread::Thread,
    time::NSEC_PER_SEC,
    vm::vmar::RssType,
};

/// Represents the inode at either `/proc/[pid]/stat` or `/proc/[pid]/task/[tid]/stat`.
///
/// The fields are the same as the ones in `/proc/[pid]/status`, but the format is different.
/// See <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/array.c#L467>.
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
/// - priority         : Kernel scheduling priority.
/// - nice             : Nice value (-20 to 19; lower is higher priority).
/// - num_threads      : Number of threads.
/// - itrealvalue      : Time before the next `SIGALRM`.
/// - starttime        : Process start time since boot (clock ticks).
/// - vsize            : Virtual memory size (bytes).
/// - rss              : Resident Set Size (pages in real memory).
/// - rsslim           : Soft memory limit (bytes).
/// - startcode        : Start address of executable code.
/// - endcode          : End address of executable code.
/// - startstack       : Initial top address of process stack.
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
    dir: TidDirOps,
    mode: StatMode,
}

#[derive(Clone, Copy)]
enum StatMode {
    Process,
    Thread,
}

impl StatFileOps {
    pub fn new_thread_inode(dir: &TidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        Self::new_inode_with_mode(dir.clone(), StatMode::Thread, parent)
    }

    pub fn new_process_inode(dir: &PidDirOps, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        Self::new_inode_with_mode(dir.tid_dir_ops().clone(), StatMode::Process, parent)
    }

    fn new_inode_with_mode(
        dir: TidDirOps,
        mode: StatMode,
        parent: Weak<dyn Inode>,
    ) -> Arc<dyn Inode> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3341>
        ProcFile::new(Self { dir, mode }, parent, mkmod!(a+r))
    }
}

impl ProcFileOps for StatFileOps {
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        self.dir.thread()
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let Some((thread, process)) = self.dir.thread_and_process() else {
            return_errno_with_message!(Errno::ESRCH, "the thread or the process does not exist");
        };
        let posix_thread = thread.as_posix_thread().unwrap();

        // According to the Linux implementation, a process's `/proc/<pid>/stat` should be
        // almost identical to its main thread's `/proc/<pid>/task/<pid>/stat`, except for
        // fields `exit_code`, `wchan`, `min_flt`, `maj_flt`, `gtime`, `utime`, and `stime`.
        //
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/array.c#L467-L681>

        let pid = posix_thread.tid();

        let comm = posix_thread
            .thread_name()
            .lock()
            .name()
            .to_string_lossy()
            .into_owned();
        let state = if thread.is_exited() {
            'Z'
        } else {
            match posix_thread.sleeping_state() {
                SleepingState::Running => 'R',
                SleepingState::Interruptible => 'S',
                SleepingState::Uninterruptible => 'D',
                SleepingState::StopBySignal => 'T',
                SleepingState::StopByPtrace => 't',
            }
        };
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

        // Placeholder: kernel flags are not exposed yet.
        let flags = 0;
        // Placeholder: page-fault counters are not tracked per process/thread yet.
        let min_flt = 0;
        let cmin_flt = 0;
        let maj_flt = 0;
        let cmaj_flt = 0;

        let (utime, stime) = {
            let prof_clock = match self.mode {
                StatMode::Process => process.prof_clock(),
                StatMode::Thread => posix_thread.prof_clock(),
            };
            (
                prof_clock.user_clock().read_jiffies().as_u64(),
                prof_clock.kernel_clock().read_jiffies().as_u64(),
            )
        };

        let (cutime, cstime) = {
            let (user_time, kernel_time) = process.reaped_children_stats().lock().get();
            (
                duration_to_jiffies(user_time),
                duration_to_jiffies(kernel_time),
            )
        };

        let (priority, rt_priority, policy) = sched_values(&thread);
        let nice = process.nice().load(Ordering::Relaxed).value().get();
        let num_threads = process.tasks().lock().as_slice().len();
        let itrealvalue =
            duration_to_jiffies(process.timer_manager().alarm_timer().lock().remain());
        let starttime = process.start_time().as_u64();

        let (
            vsize,
            rss,
            startcode,
            endcode,
            startstack,
            start_data,
            end_data,
            start_brk,
            arg_start,
            arg_end,
            env_start,
            env_end,
        ) = if let Some(vmar_ref) = process.lock_vmar().as_ref() {
            let vsize = vmar_ref.get_mappings_total_size();
            let anon = vmar_ref.get_rss_counter(RssType::Anon);
            let file = vmar_ref.get_rss_counter(RssType::File);
            let rss = anon + file;
            let process_vm = vmar_ref.process_vm();
            let code_range = process_vm.code_range();
            let data_range = process_vm.data_range();
            let init_stack = process_vm.init_stack();
            let argv_range = init_stack.argv_range();
            let envp_range = init_stack.envp_range();
            let start_brk = process_vm.heap().lock().heap_low();

            (
                vsize,
                rss,
                code_range.start,
                code_range.end,
                init_stack.user_stack_top(),
                data_range.start,
                data_range.end,
                start_brk,
                argv_range.start,
                argv_range.end,
                envp_range.start,
                envp_range.end,
            )
        } else {
            (0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)
        };

        let rsslim = process
            .resource_limits()
            .get_rlimit(ResourceType::RLIMIT_RSS)
            .get_cur();

        // Placeholder: saved SP/IP and wait-channel addresses are not exposed by `Thread`.
        let kstkesp = 0;
        let kstkeip = 0;
        let wchan = 0;

        let signal = u64::from(posix_thread.pending_signals());
        let blocked = u64::from(posix_thread.sig_mask());
        let (sigignore, sigcatch) = signal_disposition_masks(&process);

        // Placeholder: swap accounting is not implemented yet.
        let nswap = 0;
        let cnswap = 0;

        let exit_signal = process
            .exit_signal()
            .map(|sig_num| sig_num.as_u8() as i32)
            .unwrap_or(0);
        let processor = thread
            .sched_attr()
            .last_cpu()
            .map(|cpu| u32::from(cpu) as usize)
            .unwrap_or(0);

        // Placeholder: block I/O delay and guest-time accounting are not implemented yet.
        let delayacct_blkio_ticks = 0;
        let guest_time = 0;
        let cguest_time = 0;

        let exit_code = match self.mode {
            StatMode::Process => process.status().exit_code(),
            StatMode::Thread => posix_thread.exit_code(),
        };

        let mut printer = VmPrinter::new_skip(writer, offset);
        writeln!(
            printer,
            "{} ({}) {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {}",
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
            rss,
            rsslim,
            startcode,
            endcode,
            startstack,
            kstkesp,
            kstkeip,
            signal,
            blocked,
            sigignore,
            sigcatch,
            wchan,
            nswap,
            cnswap,
            exit_signal,
            processor,
            rt_priority,
            policy,
            delayacct_blkio_ticks,
            guest_time,
            cguest_time,
            start_data,
            end_data,
            start_brk,
            arg_start,
            arg_end,
            env_start,
            env_end,
            exit_code
        )?;

        Ok(printer.bytes_written())
    }
}

/// Converts a duration into kernel clock ticks.
fn duration_to_jiffies(duration: Duration) -> u64 {
    const NSEC_PER_JIFFY: u64 = NSEC_PER_SEC as u64 / TIMER_FREQ;
    const { assert!((NSEC_PER_SEC as u64).is_multiple_of(TIMER_FREQ)) };

    let sec_jiffies = duration.as_secs().saturating_mul(TIMER_FREQ);
    let subsec_jiffies = u64::from(duration.subsec_nanos()) / NSEC_PER_JIFFY;
    sec_jiffies.saturating_add(subsec_jiffies)
}

/// Returns the `priority`, `rt_priority`, and `policy` values for `/proc/<pid>/stat`.
fn sched_values(thread: &Thread) -> (i32, u8, i32) {
    const MAX_RT_PRIORITY: u8 = RealTimePriority::MAX.get();
    const RT_PRIORITY_LIMIT: u8 = MAX_RT_PRIORITY + 1;
    const NICE_TO_PRIORITY_OFFSET: i32 = 20;

    let policy = thread.sched_attr().policy();

    let (priority, rt_priority) = match policy {
        SchedPolicy::Stop => (-i32::from(MAX_RT_PRIORITY) - 1, MAX_RT_PRIORITY),
        SchedPolicy::RealTime { rt_prio, .. } => {
            // `SchedPolicy` stores real-time priorities in the scheduler's
            // internal order, where smaller values have higher priority.
            // This is the reverse of Linux's user-visible RT priority
            // used by `/proc/<pid>/stat`.
            // For example, an internal RT priority of 1 is reported as 99.
            // FIXME: Use the same conversion helper (i.e., `rt_to_static`)
            // as the `sched*` syscalls once it is available outside the
            // `syscall` module.
            let rt_priority = RT_PRIORITY_LIMIT - rt_prio.get();
            (-i32::from(rt_priority) - 1, rt_priority)
        }
        SchedPolicy::Fair(nice) => (NICE_TO_PRIORITY_OFFSET + nice.value().get() as i32, 0),
        SchedPolicy::Idle => (NICE_TO_PRIORITY_OFFSET, 0),
    };
    let linux_policy = LinuxSchedPolicy::from(policy);

    (priority, rt_priority, linux_policy as i32)
}

/// Returns the ignored and caught standard-signal masks for `/proc/<pid>/stat`.
fn signal_disposition_masks(process: &Process) -> (u64, u64) {
    let dispositions = process.sig_dispositions().lock();
    let dispositions = dispositions.lock();
    let mut ignored = 0_u64;
    let mut caught = 0_u64;

    for (sig_num, sig_action) in dispositions.iter() {
        // Linux only exposes standard signals in `sigignore` and `sigcatch`.
        if !sig_num.is_std() {
            break;
        }

        let bit = u64::from(SigMask::from(sig_num));
        match sig_action {
            SigAction::Dfl => {}
            // `sigignore` tracks explicitly ignored signals, while `sigcatch` tracks
            // signals with user-registered handlers.
            SigAction::Ign => ignored |= bit,
            SigAction::User { .. } => caught |= bit,
        }
    }

    (ignored, caught)
}
