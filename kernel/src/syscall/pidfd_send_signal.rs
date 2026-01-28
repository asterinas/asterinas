// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use crate::{
    fs::file_table::{FileDesc, get_file_fast},
    prelude::*,
    process::{
        Pgid, Pid, PidFile, kill, kill_group,
        posix_thread::AsPosixThread,
        signal::{
            c_types::siginfo_t,
            constants::SI_TKILL,
            sig_num::SigNum,
            signals::{Signal, raw::RawSignal, user::UserSignal},
        },
        tgkill,
    },
    syscall::SyscallReturn,
    thread::Tid,
};

pub fn sys_pidfd_send_signal(
    pidfd: FileDesc,
    sig_num: u64,
    info_ptr: Vaddr,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let flags = PidfdSendSignalFlags::try_from(flags)?;
    let sig_num = SigNum::try_from(sig_num as u8)?;
    debug!(
        "pidfd={}, info_ptr={:#x}, flags={:?}",
        pidfd, info_ptr, flags
    );

    let siginfo = read_siginfo_from_user(info_ptr, sig_num, ctx)?;
    let signal = RawSignal::new(siginfo);

    let target = get_target_from_pidfd(pidfd, flags, ctx)?;

    let is_self = match &target {
        SignalTarget::Thread { tid, tgid: _ } => *tid == ctx.posix_thread.tid(),
        SignalTarget::Process { pid } => *pid == ctx.posix_thread.tid(),
        SignalTarget::ProcessGroup { pgid: _ } => false,
    };

    if !is_self && (siginfo.si_code >= 0 || siginfo.si_code == SI_TKILL) {
        return_errno_with_message!(
            Errno::EPERM,
            "signals with custom code can only be sent to the current thread/process"
        );
    }

    match target {
        SignalTarget::Thread { tid, tgid } => {
            tgkill(tid, tgid, Some(Box::new(signal) as Box<dyn Signal>), ctx)?;
        }
        SignalTarget::Process { pid } => {
            kill(pid, Some(Box::new(signal) as Box<dyn Signal>), ctx)?;
        }
        SignalTarget::ProcessGroup { pgid } => {
            kill_group(pgid, Some(signal), ctx)?;
        }
    }

    Ok(SyscallReturn::Return(0))
}

fn read_siginfo_from_user(info_ptr: Vaddr, sig_num: SigNum, ctx: &Context) -> Result<siginfo_t> {
    if info_ptr != 0 {
        let si = ctx.user_space().read_val::<siginfo_t>(info_ptr)?;
        if si.si_signo != sig_num.as_u8() as i32 {
            return_errno_with_message!(
                Errno::EINVAL,
                "`siginfo.si_signo` does not match the specified signal number"
            );
        }
        Ok(si)
    } else {
        // If `info_ptr` is NULL, the kernel constructs a default `siginfo_t` structure
        // whose fields match the values that are implicitly supplied when a signal is sent using the kill(2).
        Ok(UserSignal::new_kill(sig_num, ctx).to_info())
    }
}

fn get_target_from_pidfd(
    pidfd: FileDesc,
    flags: PidfdSendSignalFlags,
    ctx: &Context,
) -> Result<SignalTarget> {
    // Helper closures to create signal targets.
    let thread_target = |tid: Tid, tgid: Pid| SignalTarget::Thread { tid, tgid };
    let process_target = |pid: Pid| SignalTarget::Process { pid };
    let group_target = |pgid: Pgid| SignalTarget::ProcessGroup { pgid };

    let target = match pidfd {
        PIDFD_SELF_THREAD => match flags {
            PidfdSendSignalFlags::Default | PidfdSendSignalFlags::Thread => {
                thread_target(ctx.posix_thread.tid(), ctx.process.pid())
            }
            PidfdSendSignalFlags::ThreadGroup => process_target(ctx.process.pid()),
            PidfdSendSignalFlags::ProcessGroup => group_target(ctx.posix_thread.tid()),
        },
        PIDFD_SELF_THREAD_GROUP => match flags {
            PidfdSendSignalFlags::Default | PidfdSendSignalFlags::ThreadGroup => {
                process_target(ctx.process.pid())
            }
            PidfdSendSignalFlags::Thread => thread_target(
                ctx.process.main_thread().as_posix_thread().unwrap().tid(),
                ctx.process.pid(),
            ),
            PidfdSendSignalFlags::ProcessGroup => group_target(ctx.process.pid()),
        },
        _ => {
            let mut file_table = ctx.thread_local.borrow_file_table_mut();
            let file = get_file_fast!(&mut file_table, pidfd);

            // FIXME: On Linux, a pidfd can be also obtained by opening a `/proc/pid` directory.
            // Reference: <https://man7.org/linux/man-pages/man2/pidfd_send_signal.2.html>
            let Some(pid_file) = file.downcast_ref::<PidFile>() else {
                return_errno_with_message!(Errno::EBADF, "the file is not a PID file");
            };

            let Some(process) = pid_file.process_opt() else {
                return_errno_with_message!(Errno::ESRCH, "the target process has been reaped");
            };

            match flags {
                PidfdSendSignalFlags::Default => {
                    // FIXME: On Linux, a pidfd can refer to either a process or a thread.
                    // We currently only support pidfds that refer to processes.
                    process_target(process.pid())
                }
                PidfdSendSignalFlags::Thread => {
                    // FIXME: On Linux, the signal can be sent to any thread.
                    // We currently only support the main thread.
                    thread_target(
                        process.main_thread().as_posix_thread().unwrap().tid(),
                        process.pid(),
                    )
                }
                PidfdSendSignalFlags::ThreadGroup => process_target(process.pid()),
                PidfdSendSignalFlags::ProcessGroup => group_target(process.pid()),
            }
        }
    };

    Ok(target)
}

enum SignalTarget {
    Thread { tid: Tid, tgid: Pid },
    Process { pid: Pid },
    ProcessGroup { pgid: Pgid },
}

// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/pidfd.h#L19>.
#[derive(Clone, Copy, PartialEq, Eq, Debug, TryFromInt)]
#[repr(u32)]
enum PidfdSendSignalFlags {
    Default = 0x0,
    Thread = 0x1,
    ThreadGroup = 0x2,
    ProcessGroup = 0x4,
}

// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/fcntl.h#L110>.
const PIDFD_SELF_THREAD: i32 = -10000;
const PIDFD_SELF_THREAD_GROUP: i32 = -10001;
