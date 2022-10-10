//! Read the Cpu context content then dispatch syscall to corrsponding handler
//! The each sub module contains functions that handle real syscall logic.

use alloc::borrow::ToOwned;
use kxos_frame::cpu::CpuContext;
use kxos_frame::{debug, warn};

use crate::process::task::HandlerResult;
use crate::syscall::arch_prctl::sys_arch_prctl;
use crate::syscall::brk::sys_brk;
use crate::syscall::exit::sys_exit;
use crate::syscall::exit_group::sys_exit_group;
use crate::syscall::fork::sys_fork;
use crate::syscall::fstat::sys_fstat;
use crate::syscall::futex::sys_futex;
use crate::syscall::getpid::sys_getpid;
use crate::syscall::gettid::sys_gettid;
use crate::syscall::mmap::sys_mmap;
use crate::syscall::mprotect::sys_mprotect;
use crate::syscall::readlink::sys_readlink;
use crate::syscall::tgkill::sys_tgkill;
use crate::syscall::uname::sys_uname;
use crate::syscall::wait4::sys_wait4;
use crate::syscall::waitid::sys_waitid;
use crate::syscall::write::sys_write;
use crate::syscall::writev::sys_writev;

mod arch_prctl;
mod brk;
mod exit;
mod exit_group;
mod fork;
mod fstat;
mod futex;
mod getpid;
mod gettid;
pub mod mmap;
mod mprotect;
mod readlink;
mod sched_yield;
mod tgkill;
mod uname;
mod wait4;
mod waitid;
mod write;
mod writev;

const SYS_WRITE: u64 = 1;
const SYS_FSTAT: u64 = 5;
const SYS_MMAP: u64 = 9;
const SYS_MPROTECT: u64 = 10;
const SYS_BRK: u64 = 12;
const SYS_RT_SIGACTION: u64 = 13;
const SYS_RT_SIGPROCMASK: u64 = 14;
const SYS_WRITEV: u64 = 20;
const SYS_SCHED_YIELD: u64 = 24;
const SYS_GETPID: u64 = 39;
const SYS_FORK: u64 = 57;
const SYS_EXIT: u64 = 60;
const SYS_WAIT4: u64 = 61;
const SYS_UNAME: u64 = 63;
const SYS_READLINK: u64 = 89;
const SYS_GETUID: u64 = 102;
const SYS_GETGID: u64 = 104;
const SYS_GETEUID: u64 = 107;
const SYS_GETEGID: u64 = 108;
const SYS_ARCH_PRCTL: u64 = 158;
const SYS_GETTID: u64 = 186;
const SYS_FUTEX: u64 = 202;
const SYS_EXIT_GROUP: u64 = 231;
const SYS_TGKILL: u64 = 234;
const SYS_WAITID: u64 = 247;

pub struct SyscallArgument {
    syscall_number: u64,
    args: [u64; 6],
}

pub enum SyscallResult {
    Exit(i32),
    Return(i32),
}

impl SyscallArgument {
    fn new_from_context(context: &CpuContext) -> Self {
        let syscall_number = context.gp_regs.rax;
        let mut args = [0u64; 6];
        args[0] = context.gp_regs.rdi;
        args[1] = context.gp_regs.rsi;
        args[2] = context.gp_regs.rdx;
        args[3] = context.gp_regs.r10;
        args[4] = context.gp_regs.r8;
        args[5] = context.gp_regs.r9;
        Self {
            syscall_number,
            args,
        }
    }
}

pub fn syscall_handler(context: &mut CpuContext) -> HandlerResult {
    let syscall_frame = SyscallArgument::new_from_context(context);
    let syscall_return =
        syscall_dispatch(syscall_frame.syscall_number, syscall_frame.args, context);

    match syscall_return {
        SyscallResult::Return(return_value) => {
            // FIXME: set return value?
            context.gp_regs.rax = return_value as u64;
            HandlerResult::Continue
        }
        SyscallResult::Exit(exit_code) => HandlerResult::Exit,
    }
}

pub fn syscall_dispatch(
    syscall_number: u64,
    args: [u64; 6],
    context: &mut CpuContext,
) -> SyscallResult {
    match syscall_number {
        SYS_WRITE => sys_write(args[0], args[1], args[2]),
        SYS_FSTAT => sys_fstat(args[0], args[1]),
        SYS_MMAP => sys_mmap(args[0], args[1], args[2], args[3], args[4], args[5]),
        SYS_MPROTECT => sys_mprotect(args[0], args[1], args[2]),
        SYS_BRK => sys_brk(args[0]),
        SYS_RT_SIGACTION => sys_rt_sigaction(),
        SYS_RT_SIGPROCMASK => sys_rt_sigprocmask(),
        SYS_WRITEV => sys_writev(args[0], args[1], args[2]),
        SYS_GETPID => sys_getpid(),
        SYS_FORK => sys_fork(context.to_owned()),
        SYS_EXIT => sys_exit(args[0] as _),
        SYS_WAIT4 => sys_wait4(args[0], args[1], args[2]),
        SYS_UNAME => sys_uname(args[0]),
        SYS_READLINK => sys_readlink(args[0], args[1], args[2]),
        SYS_GETUID => sys_getuid(),
        SYS_GETGID => sys_getgid(),
        SYS_GETEUID => sys_geteuid(),
        SYS_GETEGID => sys_getegid(),
        SYS_ARCH_PRCTL => sys_arch_prctl(args[0], args[1], context),
        SYS_GETTID => sys_gettid(),
        SYS_FUTEX => sys_futex(args[0], args[1], args[2], args[3], args[4], args[5]),
        SYS_EXIT_GROUP => sys_exit_group(args[0]),
        SYS_TGKILL => sys_tgkill(args[0], args[1], args[2]),
        SYS_WAITID => sys_waitid(args[0], args[1], args[2], args[3], args[4]),
        _ => panic!("Unsupported syscall number: {}", syscall_number),
    }
}

pub fn sys_rt_sigaction() -> SyscallResult {
    debug!("[syscall][id={}][SYS_RT_SIGACTION]", SYS_RT_SIGACTION);
    warn!("TODO: rt_sigaction only return a fake result");
    SyscallResult::Return(0)
}

pub fn sys_rt_sigprocmask() -> SyscallResult {
    debug!("[syscall][id={}][SYS_RT_SIGPROCMASK]", SYS_RT_SIGPROCMASK);
    warn!("TODO: rt_sigprocmask only return a fake result");
    SyscallResult::Return(0)
}

pub fn sys_getuid() -> SyscallResult {
    debug!("[syscall][id={}][SYS_GETUID]", SYS_GETUID);
    warn!("TODO: getuid only return a fake uid now");
    SyscallResult::Return(0)
}

pub fn sys_getgid() -> SyscallResult {
    debug!("[syscall][id={}][SYS_GETGID]", SYS_GETUID);
    warn!("TODO: getgid only return a fake gid now");
    SyscallResult::Return(0)
}

pub fn sys_geteuid() -> SyscallResult {
    debug!("[syscall][id={}][SYS_GETEUID]", SYS_GETEUID);
    warn!("TODO: geteuid only return a fake euid now");
    SyscallResult::Return(0)
}

pub fn sys_getegid() -> SyscallResult {
    debug!("[syscall][id={}][SYS_GETEGID]", SYS_GETEGID);
    warn!("TODO: getegid only return a fake egid now");
    SyscallResult::Return(0)
}
