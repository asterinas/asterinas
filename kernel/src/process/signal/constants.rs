// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

/// Standard signals
pub(super) const MIN_STD_SIG_NUM: u8 = 1;
pub(super) const MAX_STD_SIG_NUM: u8 = 31; // inclusive
/// Real-time signals
pub(super) const MIN_RT_SIG_NUM: u8 = 32;
pub(super) const MAX_RT_SIG_NUM: u8 = 64; // inclusive
/// Count the number of signals
pub(super) const COUNT_STD_SIGS: usize = 31;
pub(super) const COUNT_RT_SIGS: usize = 33;
pub(super) const COUNT_ALL_SIGS: usize = 64;

pub const SIG_DFL: usize = 0;
pub const SIG_IGN: usize = 1;

use super::sig_num::SigNum;

macro_rules! define_std_signums {
    ( $( $name: ident = $num: expr ),+, ) => {
        $(
            pub const $name : SigNum = SigNum::from_u8($num);
        )*
    }
}

define_std_signums! {
    SIGHUP    = 1, // Hangup detected on controlling terminal or death of controlling process
    SIGINT    = 2, // Interrupt from keyboard
    SIGQUIT   = 3, // Quit from keyboard
    SIGILL    = 4, // Illegal Instruction
    SIGTRAP   = 5, // Trace/breakpoint trap
    SIGABRT   = 6, // Abort signal from abort(3)
    SIGBUS    = 7, // Bus error (bad memory access)
    SIGFPE    = 8, // Floating-point exception
    SIGKILL   = 9, // Kill signal
    SIGUSR1   = 10, // User-defined signal 1
    SIGSEGV   = 11, // Invalid memory reference
    SIGUSR2   = 12, // User-defined signal 2
    SIGPIPE   = 13, // Broken pipe: write to pipe with no readers; see pipe(7)
    SIGALRM   = 14, // Timer signal from alarm(2)
    SIGTERM   = 15, // Termination signal
    SIGSTKFLT = 16, // Stack fault on coprocessor (unused)
    SIGCHLD   = 17, // Child stopped or terminated
    SIGCONT   = 18, // Continue if stopped
    SIGSTOP   = 19, // Stop process
    SIGTSTP   = 20, // Stop typed at terminal
    SIGTTIN   = 21, // Terminal input for background process
    SIGTTOU   = 22, // Terminal output for background process
    SIGURG    = 23, // Urgent condition on socket (4.2BSD)
    SIGXCPU   = 24, // CPU time limit exceeded (4.2BSD); see setrlimit(2)
    SIGXFSZ   = 25, // File size limit exceeded (4.2BSD); see setrlimit(2)
    SIGVTALRM = 26, // Virtual alarm clock (4.2BSD)
    SIGPROF   = 27, // Profiling timer expired
    SIGWINCH  = 28, // Window resize signal (4.3BSD, Sun)
    SIGIO     = 29, // I/O now possible (4.2BSD)
    SIGPWR    = 30, // Power failure (System V)
    SIGSYS    = 31, // Bad system call (SVr4); see also seccomp(2)
}

pub const SI_ASYNCNL: i32 = -60;
pub const SI_TKILL: i32 = -6;
pub const SI_SIGIO: i32 = -5;
pub const SI_ASYNCIO: i32 = -4;
pub const SI_MESGQ: i32 = -3;
pub const SI_TIMER: i32 = -2;
pub const SI_QUEUE: i32 = -1;
pub const SI_USER: i32 = 0;
pub const SI_KERNEL: i32 = 128;

pub const FPE_INTDIV: i32 = 1;
pub const FPE_INTOVF: i32 = 2;
pub const FPE_FLTDIV: i32 = 3;
pub const FPE_FLTOVF: i32 = 4;
pub const FPE_FLTUND: i32 = 5;
pub const FPE_FLTRES: i32 = 6;
pub const FPE_FLTINV: i32 = 7;
pub const FPE_FLTSUB: i32 = 8;

pub const ILL_ILLOPC: i32 = 1;
pub const ILL_ILLOPN: i32 = 2;
pub const ILL_ILLADR: i32 = 3;
pub const ILL_ILLTRP: i32 = 4;
pub const ILL_PRVOPC: i32 = 5;
pub const ILL_PRVREG: i32 = 6;
pub const ILL_COPROC: i32 = 7;
pub const ILL_BADSTK: i32 = 8;

pub const SEGV_MAPERR: i32 = 1;
pub const SEGV_ACCERR: i32 = 2;
pub const SEGV_BNDERR: i32 = 3;
pub const SEGV_PKUERR: i32 = 4;

pub const BUS_ADRALN: i32 = 1;
pub const BUS_ADRERR: i32 = 2;
pub const BUS_OBJERR: i32 = 3;
pub const BUS_MCEERR_AR: i32 = 4;
pub const BUS_MCEERR_AO: i32 = 5;

pub const CLD_EXITED: i32 = 1;
pub const CLD_KILLED: i32 = 2;
pub const CLD_DUMPED: i32 = 3;
pub const CLD_TRAPPED: i32 = 4;
pub const CLD_STOPPED: i32 = 5;
pub const CLD_CONTINUED: i32 = 6;
