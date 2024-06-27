// SPDX-License-Identifier: MPL-2.0

//! Read the Cpu context content then dispatch syscall to corrsponding handler
//! The each sub module contains functions that handle real syscall logic.
pub use clock_gettime::ClockId;
use ostd::cpu::UserContext;

use crate::{cpu::LinuxAbi, prelude::*};

mod accept;
mod access;
mod alarm;
mod arch;
mod arch_prctl;
mod bind;
mod brk;
mod capget;
mod capset;
mod chdir;
mod chmod;
mod chown;
mod chroot;
mod clock_gettime;
mod clone;
mod close;
mod connect;
mod constants;
mod dup;
mod epoll;
mod eventfd;
mod execve;
mod exit;
mod exit_group;
mod fcntl;
mod fork;
mod fsync;
mod futex;
mod getcwd;
mod getdents64;
mod getegid;
mod geteuid;
mod getgid;
mod getgroups;
mod getpeername;
mod getpgrp;
mod getpid;
mod getppid;
mod getrandom;
mod getresgid;
mod getresuid;
mod getrusage;
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
mod mount;
mod mprotect;
mod munmap;
mod nanosleep;
mod open;
mod pause;
mod pipe;
mod poll;
mod prctl;
mod pread64;
mod preadv;
mod prlimit64;
mod pwrite64;
mod pwritev;
mod read;
mod readlink;
mod recvfrom;
mod recvmsg;
mod rename;
mod rmdir;
mod rt_sigaction;
mod rt_sigpending;
mod rt_sigprocmask;
mod rt_sigreturn;
mod rt_sigsuspend;
mod sched_getaffinity;
mod sched_yield;
mod select;
mod sendfile;
mod sendmsg;
mod sendto;
mod set_get_priority;
mod set_robust_list;
mod set_tid_address;
mod setfsgid;
mod setfsuid;
mod setgid;
mod setgroups;
mod setitimer;
mod setpgid;
mod setregid;
mod setresgid;
mod setresuid;
mod setreuid;
mod setsid;
mod setsockopt;
mod setuid;
mod shutdown;
mod sigaltstack;
mod socket;
mod socketpair;
mod stat;
mod statfs;
mod symlink;
mod sync;
mod tgkill;
mod time;
mod timer_create;
mod timer_settime;
mod truncate;
mod umask;
mod umount;
mod uname;
mod unlink;
mod utimens;
mod wait4;
mod waitid;
mod write;

/// This macro is used to define syscall handler.
/// The first param is ths number of parameters,
/// The second param is the function name of syscall handler,
/// The third is optional, means the args(if parameter number > 0),
/// The third is optional, means if cpu context is required.
macro_rules! syscall_handler {
    (0, $fn_name: ident, $args: ident) => { $fn_name() };
    (0, $fn_name: ident, $args: ident, $context: expr) => { $fn_name($context) };
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

macro_rules! dispatch_fn_inner {
    ( $args: ident, $context: ident, $handler: ident ( args[ .. $cnt: tt ] ) ) => {
        $crate::syscall::syscall_handler!($cnt, $handler, $args)
    };
    ( $args: ident, $context: ident, $handler: ident ( args[ .. $cnt: tt ] , &context ) ) => {
        $crate::syscall::syscall_handler!($cnt, $handler, $args, &$context)
    };
    ( $args: ident, $context: ident, $handler: ident ( args[ .. $cnt: tt ] , &mut context ) ) => {
        // `$context` is already of type `&mut ostd::cpu::UserContext`,
        // so no need to take `&mut` again
        $crate::syscall::syscall_handler!($cnt, $handler, $args, $context)
    };
}

macro_rules! impl_syscall_nums_and_dispatch_fn {
    // $args, $context, and $dispatcher_name are needed since Rust macro is hygienic
    ( $( $name: ident = $num: literal => $handler: ident $args: tt );* $(;)? ) => {
        // First, define the syscall numbers
        $(
            pub const $name: u64 = $num;
        )*

        // Then, define the dispatcher function
        pub fn syscall_dispatch(
            syscall_number: u64,
            args: [u64; 6],
            context: &mut ostd::cpu::UserContext,
        ) -> $crate::prelude::Result<$crate::syscall::SyscallReturn> {
            match syscall_number {
                $(
                    $num => {
                        $crate::log_syscall_entry!($name);
                        $crate::syscall::dispatch_fn_inner!(args, context, $handler $args)
                    }
                )*
                _ => {
                    log::warn!("Unimplemented syscall number: {}", syscall_number);
                    $crate::return_errno_with_message!($crate::error::Errno::ENOSYS, "Syscall was unimplemented");
                }
            }
        }
    }
}

// Export macros to sub-modules
use dispatch_fn_inner;
use impl_syscall_nums_and_dispatch_fn;
use syscall_handler;

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
        let syscall_number = context.syscall_num() as u64;
        let args = context.syscall_args().map(|x| x as u64);
        Self {
            syscall_number,
            args,
        }
    }
}

pub fn handle_syscall(context: &mut UserContext) {
    let syscall_frame = SyscallArgument::new_from_context(context);
    let syscall_return =
        arch::syscall_dispatch(syscall_frame.syscall_number, syscall_frame.args, context);

    match syscall_return {
        Ok(return_value) => {
            if let SyscallReturn::Return(return_value) = return_value {
                context.set_syscall_ret(return_value as usize);
            }
        }
        Err(err) => {
            debug!("syscall return error: {:?}", err);
            let errno = err.error() as i32;
            context.set_syscall_ret((-errno) as usize)
        }
    }
}

#[macro_export]
macro_rules! log_syscall_entry {
    ($syscall_name: tt) => {
        if log::log_enabled!(log::Level::Info) {
            let syscall_name_str = stringify!($syscall_name);
            let pid = $crate::current!().pid();
            let tid = $crate::current_thread!().tid();
            log::info!(
                "[pid={}][tid={}][id={}][{}]",
                pid,
                tid,
                $syscall_name,
                syscall_name_str
            );
        }
    };
}
