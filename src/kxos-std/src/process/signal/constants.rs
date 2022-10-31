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
