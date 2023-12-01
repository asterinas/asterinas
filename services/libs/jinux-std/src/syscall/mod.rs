//! Read the Cpu context content then dispatch syscall to corrsponding handler
//! The each sub module contains functions that handle real syscall logic.
use crate::prelude::*;
use crate::syscall::access::sys_access;
use crate::syscall::arch_prctl::sys_arch_prctl;
use crate::syscall::brk::sys_brk;
use crate::syscall::chdir::{sys_chdir, sys_fchdir};
use crate::syscall::chmod::{sys_chmod, sys_fchmod, sys_fchmodat};
use crate::syscall::clock_gettime::sys_clock_gettime;
use crate::syscall::clock_nanosleep::sys_clock_nanosleep;
use crate::syscall::clone::sys_clone;
use crate::syscall::close::sys_close;
use crate::syscall::dup::{sys_dup, sys_dup2};
use crate::syscall::epoll::{sys_epoll_create, sys_epoll_create1, sys_epoll_ctl, sys_epoll_wait};
use crate::syscall::execve::sys_execve;
use crate::syscall::exit::sys_exit;
use crate::syscall::exit_group::sys_exit_group;
use crate::syscall::fcntl::sys_fcntl;
use crate::syscall::fork::sys_fork;
use crate::syscall::futex::sys_futex;
use crate::syscall::getcwd::sys_getcwd;
use crate::syscall::getdents64::sys_getdents64;
use crate::syscall::getegid::sys_getegid;
use crate::syscall::geteuid::sys_geteuid;
use crate::syscall::getgid::sys_getgid;
use crate::syscall::getpgrp::sys_getpgrp;
use crate::syscall::getpid::sys_getpid;
use crate::syscall::getppid::sys_getppid;
use crate::syscall::gettid::sys_gettid;
use crate::syscall::gettimeofday::sys_gettimeofday;
use crate::syscall::getuid::sys_getuid;
use crate::syscall::ioctl::sys_ioctl;
use crate::syscall::kill::sys_kill;
use crate::syscall::link::{sys_link, sys_linkat};
use crate::syscall::lseek::sys_lseek;
use crate::syscall::madvise::sys_madvise;
use crate::syscall::mkdir::{sys_mkdir, sys_mkdirat};
use crate::syscall::mmap::sys_mmap;
use crate::syscall::mprotect::sys_mprotect;
use crate::syscall::munmap::sys_munmap;
use crate::syscall::open::{sys_open, sys_openat};
use crate::syscall::pause::sys_pause;
use crate::syscall::pipe::{sys_pipe, sys_pipe2};
use crate::syscall::poll::sys_poll;
use crate::syscall::prctl::sys_prctl;
use crate::syscall::prlimit64::sys_prlimit64;
use crate::syscall::read::sys_read;
use crate::syscall::readlink::{sys_readlink, sys_readlinkat};
use crate::syscall::rename::{sys_rename, sys_renameat};
use crate::syscall::rmdir::sys_rmdir;
use crate::syscall::rt_sigaction::sys_rt_sigaction;
use crate::syscall::rt_sigprocmask::sys_rt_sigprocmask;
use crate::syscall::rt_sigreturn::sys_rt_sigreturn;
use crate::syscall::sched_yield::sys_sched_yield;
use crate::syscall::select::sys_select;
use crate::syscall::set_robust_list::sys_set_robust_list;
use crate::syscall::set_tid_address::sys_set_tid_address;
use crate::syscall::setpgid::sys_setpgid;
use crate::syscall::stat::{sys_fstat, sys_fstatat, sys_lstat, sys_stat};
use crate::syscall::statfs::{sys_fstatfs, sys_statfs};
use crate::syscall::symlink::{sys_symlink, sys_symlinkat};
use crate::syscall::tgkill::sys_tgkill;
use crate::syscall::time::sys_time;
use crate::syscall::umask::sys_umask;
use crate::syscall::uname::sys_uname;
use crate::syscall::unlink::{sys_unlink, sys_unlinkat};
use crate::syscall::utimens::sys_utimensat;
use crate::syscall::wait4::sys_wait4;
use crate::syscall::waitid::sys_waitid;
use crate::syscall::write::sys_write;
use crate::syscall::writev::sys_writev;
use jinux_frame::cpu::UserContext;

use self::accept::sys_accept;
use self::bind::sys_bind;
use self::connect::sys_connect;
use self::execve::sys_execveat;
use self::getpeername::sys_getpeername;
use self::getrandom::sys_getrandom;
use self::getsid::sys_getsid;
use self::getsockname::sys_getsockname;
use self::getsockopt::sys_getsockopt;
use self::listen::sys_listen;
use self::pread64::sys_pread64;
use self::recvfrom::sys_recvfrom;
use self::sendto::sys_sendto;
use self::setsid::sys_setsid;
use self::setsockopt::sys_setsockopt;
use self::shutdown::sys_shutdown;
use self::socket::sys_socket;
use self::socketpair::sys_socketpair;

mod accept;
mod access;
mod arch_prctl;
mod bind;
mod brk;
mod chdir;
mod chmod;
mod clock_gettime;
mod clock_nanosleep;
mod clone;
mod close;
mod connect;
mod constants;
mod dup;
mod epoll;
mod execve;
mod exit;
mod exit_group;
mod fcntl;
mod fork;
mod futex;
mod getcwd;
mod getdents64;
mod getegid;
mod geteuid;
mod getgid;
mod getpeername;
mod getpgrp;
mod getpid;
mod getppid;
mod getrandom;
mod getsid;
mod getsockname;
mod getsockopt;
mod gettid;
mod gettimeofday;
mod getuid;
mod ioctl;
mod kill;
mod link;
mod listen;
mod lseek;
mod madvise;
mod mkdir;
mod mmap;
mod mprotect;
mod munmap;
mod open;
mod pause;
mod pipe;
mod poll;
mod prctl;
mod pread64;
mod prlimit64;
mod read;
mod readlink;
mod recvfrom;
mod rename;
mod rmdir;
mod rt_sigaction;
mod rt_sigprocmask;
mod rt_sigreturn;
mod sched_yield;
mod select;
mod sendto;
mod set_robust_list;
mod set_tid_address;
mod setpgid;
mod setsid;
mod setsockopt;
mod shutdown;
mod socket;
mod socketpair;
mod stat;
mod statfs;
mod symlink;
mod tgkill;
mod time;
mod umask;
mod uname;
mod unlink;
mod utimens;
mod wait4;
mod waitid;
mod write;
mod writev;

macro_rules! define_syscall_nums {
    ( $( $name: ident = $num: expr ),+ ) => {
        $(
            const $name: u64  = $num;
        )*
    }
}

/// This macro is used to define syscall handler.
/// The first param is ths number of parameters,
/// The second param is the function name of syscall handler,
/// The third is optional, means the args(if parameter number > 0),
/// The third is optional, means if cpu context is required.
macro_rules! syscall_handler {
    (0, $fn_name: ident) => { $fn_name() };
    (0, $fn_name: ident, $context: expr) => { $fn_name($context) };
    (1, $fn_name: ident, $args: ident) => { $fn_name($args[0] as _) };
    (1, $fn_name: ident, $args: ident, $context: expr) => { $fn_name($args[0] as _, $context) };
    (2, $fn_name: ident, $args: ident) => { $fn_name($args[0] as _, $args[1] as _)};
    (2, $fn_name: ident, $args: ident, $context: expr) => { $fn_name($args[0] as _, $args[1] as _, $context)};
    (3, $fn_name: ident, $args: ident) => { $fn_name($args[0] as _, $args[1] as _, $args[2] as _)};
    (3, $fn_name: ident, $args: ident, $context: expr) => { $fn_name($args[0] as _, $args[1] as _, $args[2] as _, $context)};
    (4, $fn_name: ident, $args: ident) => { $fn_name($args[0] as _, $args[1] as _, $args[2] as _, $args[3] as _)};
    (4, $fn_name: ident, $args: ident, $context: expr) => { $fn_name($args[0] as _, $args[1] as _, $args[2] as _, $args[3] as _), $context};
    (5, $fn_name: ident, $args: ident) => { $fn_name($args[0] as _, $args[1] as _, $args[2] as _, $args[3] as _, $args[4] as _)};
    (5, $fn_name: ident, $args: ident, $context: expr) => { $fn_name($args[0] as _, $args[1] as _, $args[2] as _, $args[3] as _, $args[4] as _, $context)};
    (6, $fn_name: ident, $args: ident) => { $fn_name($args[0] as _, $args[1] as _, $args[2] as _, $args[3] as _, $args[4] as _, $args[5] as _)};
    (6, $fn_name: ident, $args: ident, $context: expr) => { $fn_name($args[0] as _, $args[1] as _, $args[2] as _, $args[3] as _, $args[4] as _, $args[5] as _, $context)};
}

define_syscall_nums!(
    SYS_READ = 0,
    SYS_WRITE = 1,
    SYS_OPEN = 2,
    SYS_CLOSE = 3,
    SYS_STAT = 4,
    SYS_FSTAT = 5,
    SYS_LSTAT = 6,
    SYS_POLL = 7,
    SYS_LSEEK = 8,
    SYS_MMAP = 9,
    SYS_MPROTECT = 10,
    SYS_MUNMAP = 11,
    SYS_BRK = 12,
    SYS_RT_SIGACTION = 13,
    SYS_RT_SIGPROCMASK = 14,
    SYS_RT_SIGRETRUN = 15,
    SYS_IOCTL = 16,
    SYS_PREAD64 = 17,
    SYS_WRITEV = 20,
    SYS_ACCESS = 21,
    SYS_PIPE = 22,
    SYS_SELECT = 23,
    SYS_SCHED_YIELD = 24,
    SYS_MADVISE = 28,
    SYS_DUP = 32,
    SYS_DUP2 = 33,
    SYS_PAUSE = 34,
    SYS_GETPID = 39,
    SYS_SOCKET = 41,
    SYS_CONNECT = 42,
    SYS_ACCEPT = 43,
    SYS_SENDTO = 44,
    SYS_RECVFROM = 45,
    SYS_SHUTDOWN = 48,
    SYS_BIND = 49,
    SYS_LISTEN = 50,
    SYS_GETSOCKNAME = 51,
    SYS_GETPEERNAME = 52,
    SYS_SOCKETPAIR = 53,
    SYS_SETSOCKOPT = 54,
    SYS_GETSOCKOPT = 55,
    SYS_CLONE = 56,
    SYS_FORK = 57,
    SYS_EXECVE = 59,
    SYS_EXIT = 60,
    SYS_WAIT4 = 61,
    SYS_KILL = 62,
    SYS_UNAME = 63,
    SYS_FCNTL = 72,
    SYS_GETCWD = 79,
    SYS_CHDIR = 80,
    SYS_FCHDIR = 81,
    SYS_RENAME = 82,
    SYS_MKDIR = 83,
    SYS_RMDIR = 84,
    SYS_LINK = 86,
    SYS_UNLINK = 87,
    SYS_SYMLINK = 88,
    SYS_READLINK = 89,
    SYS_CHMOD = 90,
    SYS_FCHMOD = 91,
    SYS_UMASK = 95,
    SYS_GETTIMEOFDAY = 96,
    SYS_GETUID = 102,
    SYS_GETGID = 104,
    SYS_GETEUID = 107,
    SYS_GETEGID = 108,
    SYS_SETPGID = 109,
    SYS_GETPPID = 110,
    SYS_GETPGRP = 111,
    SYS_SETSID = 112,
    SYS_GETSID = 124,
    SYS_STATFS = 137,
    SYS_FSTATFS = 138,
    SYS_PRCTL = 157,
    SYS_ARCH_PRCTL = 158,
    SYS_GETTID = 186,
    SYS_TIME = 201,
    SYS_FUTEX = 202,
    SYS_EPOLL_CREATE = 213,
    SYS_GETDENTS64 = 217,
    SYS_SET_TID_ADDRESS = 218,
    SYS_CLOCK_GETTIME = 228,
    SYS_CLOCK_NANOSLEEP = 230,
    SYS_EXIT_GROUP = 231,
    SYS_EPOLL_WAIT = 232,
    SYS_EPOLL_CTL = 233,
    SYS_TGKILL = 234,
    SYS_WAITID = 247,
    SYS_OPENAT = 257,
    SYS_MKDIRAT = 258,
    SYS_FSTATAT = 262,
    SYS_UNLINKAT = 263,
    SYS_RENAMEAT = 264,
    SYS_LINKAT = 265,
    SYS_SYMLINKAT = 266,
    SYS_READLINKAT = 267,
    SYS_FCHMODAT = 268,
    SYS_SET_ROBUST_LIST = 273,
    SYS_UTIMENSAT = 280,
    SYS_EPOLL_CREATE1 = 291,
    SYS_PIPE2 = 293,
    SYS_PRLIMIT64 = 302,
    SYS_GETRANDOM = 318,
    SYS_EXECVEAT = 322
);

pub struct SyscallArgument {
    syscall_number: u64,
    args: [u64; 6],
}

/// Syscall return
#[derive(Debug, Clone, Copy)]
pub enum SyscallReturn {
    /// return isize, this value will be used to set rax
    Return(isize),
    /// does not need to set rax
    NoReturn,
}

impl SyscallArgument {
    fn new_from_context(context: &UserContext) -> Self {
        let syscall_number = context.rax() as u64;
        let mut args = [0u64; 6];
        args[0] = context.rdi() as u64;
        args[1] = context.rsi() as u64;
        args[2] = context.rdx() as u64;
        args[3] = context.r10() as u64;
        args[4] = context.r8() as u64;
        args[5] = context.r9() as u64;
        Self {
            syscall_number,
            args,
        }
    }
}

pub fn handle_syscall(context: &mut UserContext) {
    let syscall_frame = SyscallArgument::new_from_context(context);
    let syscall_return =
        syscall_dispatch(syscall_frame.syscall_number, syscall_frame.args, context);

    match syscall_return {
        Ok(return_value) => {
            if let SyscallReturn::Return(return_value) = return_value {
                context.set_rax(return_value as usize);
            }
        }
        Err(err) => {
            debug!("syscall return error: {:?}", err);
            let errno = err.error() as i32;
            context.set_rax((-errno) as usize)
        }
    }
}

pub fn syscall_dispatch(
    syscall_number: u64,
    args: [u64; 6],
    context: &mut UserContext,
) -> Result<SyscallReturn> {
    match syscall_number {
        SYS_READ => syscall_handler!(3, sys_read, args),
        SYS_WRITE => syscall_handler!(3, sys_write, args),
        SYS_OPEN => syscall_handler!(3, sys_open, args),
        SYS_CLOSE => syscall_handler!(1, sys_close, args),
        SYS_STAT => syscall_handler!(2, sys_stat, args),
        SYS_FSTAT => syscall_handler!(2, sys_fstat, args),
        SYS_LSTAT => syscall_handler!(2, sys_lstat, args),
        SYS_POLL => syscall_handler!(3, sys_poll, args),
        SYS_LSEEK => syscall_handler!(3, sys_lseek, args),
        SYS_MMAP => syscall_handler!(6, sys_mmap, args),
        SYS_MPROTECT => syscall_handler!(3, sys_mprotect, args),
        SYS_MUNMAP => syscall_handler!(2, sys_munmap, args),
        SYS_BRK => syscall_handler!(1, sys_brk, args),
        SYS_RT_SIGACTION => syscall_handler!(4, sys_rt_sigaction, args),
        SYS_RT_SIGPROCMASK => syscall_handler!(4, sys_rt_sigprocmask, args),
        SYS_RT_SIGRETRUN => syscall_handler!(0, sys_rt_sigreturn, context),
        SYS_IOCTL => syscall_handler!(3, sys_ioctl, args),
        SYS_PREAD64 => syscall_handler!(4, sys_pread64, args),
        SYS_WRITEV => syscall_handler!(3, sys_writev, args),
        SYS_ACCESS => syscall_handler!(2, sys_access, args),
        SYS_PIPE => syscall_handler!(1, sys_pipe, args),
        SYS_SELECT => syscall_handler!(5, sys_select, args),
        SYS_SCHED_YIELD => syscall_handler!(0, sys_sched_yield),
        SYS_MADVISE => syscall_handler!(3, sys_madvise, args),
        SYS_DUP => syscall_handler!(1, sys_dup, args),
        SYS_DUP2 => syscall_handler!(2, sys_dup2, args),
        SYS_PAUSE => syscall_handler!(0, sys_pause),
        SYS_GETPID => syscall_handler!(0, sys_getpid),
        SYS_SOCKET => syscall_handler!(3, sys_socket, args),
        SYS_CONNECT => syscall_handler!(3, sys_connect, args),
        SYS_ACCEPT => syscall_handler!(3, sys_accept, args),
        SYS_SENDTO => syscall_handler!(6, sys_sendto, args),
        SYS_RECVFROM => syscall_handler!(6, sys_recvfrom, args),
        SYS_SHUTDOWN => syscall_handler!(2, sys_shutdown, args),
        SYS_BIND => syscall_handler!(3, sys_bind, args),
        SYS_LISTEN => syscall_handler!(2, sys_listen, args),
        SYS_GETSOCKNAME => syscall_handler!(3, sys_getsockname, args),
        SYS_GETPEERNAME => syscall_handler!(3, sys_getpeername, args),
        SYS_SOCKETPAIR => syscall_handler!(4, sys_socketpair, args),
        SYS_SETSOCKOPT => syscall_handler!(5, sys_setsockopt, args),
        SYS_GETSOCKOPT => syscall_handler!(5, sys_getsockopt, args),
        SYS_CLONE => syscall_handler!(5, sys_clone, args, *context),
        SYS_FORK => syscall_handler!(0, sys_fork, *context),
        SYS_EXECVE => syscall_handler!(3, sys_execve, args, context),
        SYS_EXIT => syscall_handler!(1, sys_exit, args),
        SYS_WAIT4 => syscall_handler!(3, sys_wait4, args),
        SYS_KILL => syscall_handler!(2, sys_kill, args),
        SYS_UNAME => syscall_handler!(1, sys_uname, args),
        SYS_FCNTL => syscall_handler!(3, sys_fcntl, args),
        SYS_GETCWD => syscall_handler!(2, sys_getcwd, args),
        SYS_CHDIR => syscall_handler!(1, sys_chdir, args),
        SYS_FCHDIR => syscall_handler!(1, sys_fchdir, args),
        SYS_RENAME => syscall_handler!(2, sys_rename, args),
        SYS_MKDIR => syscall_handler!(2, sys_mkdir, args),
        SYS_RMDIR => syscall_handler!(1, sys_rmdir, args),
        SYS_LINK => syscall_handler!(2, sys_link, args),
        SYS_UNLINK => syscall_handler!(1, sys_unlink, args),
        SYS_SYMLINK => syscall_handler!(2, sys_symlink, args),
        SYS_READLINK => syscall_handler!(3, sys_readlink, args),
        SYS_CHMOD => syscall_handler!(2, sys_chmod, args),
        SYS_FCHMOD => syscall_handler!(2, sys_fchmod, args),
        SYS_UMASK => syscall_handler!(1, sys_umask, args),
        SYS_GETTIMEOFDAY => syscall_handler!(1, sys_gettimeofday, args),
        SYS_GETUID => syscall_handler!(0, sys_getuid),
        SYS_GETGID => syscall_handler!(0, sys_getgid),
        SYS_GETEUID => syscall_handler!(0, sys_geteuid),
        SYS_GETEGID => syscall_handler!(0, sys_getegid),
        SYS_SETPGID => syscall_handler!(2, sys_setpgid, args),
        SYS_GETPPID => syscall_handler!(0, sys_getppid),
        SYS_GETPGRP => syscall_handler!(0, sys_getpgrp),
        SYS_SETSID => syscall_handler!(0, sys_setsid),
        SYS_GETSID => syscall_handler!(1, sys_getsid, args),
        SYS_STATFS => syscall_handler!(2, sys_statfs, args),
        SYS_FSTATFS => syscall_handler!(2, sys_fstatfs, args),
        SYS_PRCTL => syscall_handler!(5, sys_prctl, args),
        SYS_ARCH_PRCTL => syscall_handler!(2, sys_arch_prctl, args, context),
        SYS_GETTID => syscall_handler!(0, sys_gettid),
        SYS_TIME => syscall_handler!(1, sys_time, args),
        SYS_FUTEX => syscall_handler!(6, sys_futex, args),
        SYS_EPOLL_CREATE => syscall_handler!(1, sys_epoll_create, args),
        SYS_GETDENTS64 => syscall_handler!(3, sys_getdents64, args),
        SYS_SET_TID_ADDRESS => syscall_handler!(1, sys_set_tid_address, args),
        SYS_CLOCK_GETTIME => syscall_handler!(2, sys_clock_gettime, args),
        SYS_CLOCK_NANOSLEEP => syscall_handler!(4, sys_clock_nanosleep, args),
        SYS_EXIT_GROUP => syscall_handler!(1, sys_exit_group, args),
        SYS_EPOLL_WAIT => syscall_handler!(4, sys_epoll_wait, args),
        SYS_EPOLL_CTL => syscall_handler!(4, sys_epoll_ctl, args),
        SYS_TGKILL => syscall_handler!(3, sys_tgkill, args),
        SYS_WAITID => syscall_handler!(5, sys_waitid, args),
        SYS_OPENAT => syscall_handler!(4, sys_openat, args),
        SYS_MKDIRAT => syscall_handler!(3, sys_mkdirat, args),
        SYS_FSTATAT => syscall_handler!(4, sys_fstatat, args),
        SYS_UNLINKAT => syscall_handler!(3, sys_unlinkat, args),
        SYS_RENAMEAT => syscall_handler!(4, sys_renameat, args),
        SYS_LINKAT => syscall_handler!(5, sys_linkat, args),
        SYS_SYMLINKAT => syscall_handler!(3, sys_symlinkat, args),
        SYS_READLINKAT => syscall_handler!(4, sys_readlinkat, args),
        SYS_FCHMODAT => syscall_handler!(3, sys_fchmodat, args),
        SYS_SET_ROBUST_LIST => syscall_handler!(2, sys_set_robust_list, args),
        SYS_UTIMENSAT => syscall_handler!(4, sys_utimensat, args),
        SYS_EPOLL_CREATE1 => syscall_handler!(1, sys_epoll_create1, args),
        SYS_PIPE2 => syscall_handler!(2, sys_pipe2, args),
        SYS_PRLIMIT64 => syscall_handler!(4, sys_prlimit64, args),
        SYS_GETRANDOM => syscall_handler!(3, sys_getrandom, args),
        SYS_EXECVEAT => syscall_handler!(5, sys_execveat, args, context),
        _ => {
            warn!("Unimplemented syscall number: {}", syscall_number);
            return_errno_with_message!(Errno::ENOSYS, "Syscall was unimplemented");
        }
    }
}

#[macro_export]
macro_rules! log_syscall_entry {
    ($syscall_name: tt) => {
        let syscall_name_str = stringify!($syscall_name);
        info!("[SYSCALL][id={}][{}]", $syscall_name, syscall_name_str);
    };
}
