// SPDX-License-Identifier: MPL-2.0

//! Ptrace utilities for POSIX threads.

use crate::{
    prelude::*,
    process::{
        ExitCode, WaitOptions,
        signal::{
            DequeuedSignal, c_types::siginfo_t, constants::SIGTRAP, sig_num::SigNum,
            signals::Signal,
        },
    },
    thread::Tid,
};

/// The requests that can continue a stopped tracee.
pub enum PtraceContRequest {
    Continue(Option<SigNum>),
    #[cfg_attr(not(target_arch = "x86_64"), expect(dead_code))]
    SingleStep(Option<SigNum>),
    Syscall(Option<SigNum>),
}

impl PtraceContRequest {
    pub(super) fn sig_num(&self) -> Option<SigNum> {
        match self {
            Self::Continue(Some(sig_num))
            | Self::SingleStep(Some(sig_num))
            | Self::Syscall(Some(sig_num)) => Some(*sig_num),
            _ => None,
        }
    }
}

/// The result of a ptrace-stop.
pub enum PtraceStopResult {
    /// The ptrace-stop is continued by the tracer,
    /// or ends because the tracer exits or detaches.
    Continued(Option<DequeuedSignal>),
    /// The ptrace-stop is interrupted by `SIGKILL`.
    Interrupted,
    /// The thread is not traced, or the stop condition is not met
    /// (e.g., the tracee is not being traced for syscall-stops).
    /// Returning the stop signal back, if it is a signal-delivery-stop.
    NotTraced(Option<DequeuedSignal>),
}

/// The signal associated with a ptrace-stop and its later signal delivery.
#[derive(Default)]
pub(super) enum StopDeliverySignal {
    /// The signal that has not yet been reported through `wait`.
    Pending(DequeuedSignal, PtraceWaitStatus),
    /// The signal that has been reported through `wait`.
    Consumed(DequeuedSignal),
    /// The signal that is injected by the tracer.
    Injected(DequeuedSignal),
    /// No ptrace-stop signal is recorded.
    #[default]
    Empty,
}

impl StopDeliverySignal {
    /// Records the signal associated with a ptrace-stop.
    pub(super) fn stop(&mut self, signal: DequeuedSignal, wait_status: PtraceWaitStatus) {
        *self = Self::Pending(signal, wait_status);
    }

    /// Clears and returns the signal associated with a ptrace-stop,
    /// unless it has already been consumed by `wait`.
    pub(super) fn clear(&mut self) -> Option<DequeuedSignal> {
        let this = core::mem::replace(self, Self::Empty);

        match this {
            Self::Pending(signal, _) | Self::Injected(signal) => Some(signal),
            Self::Consumed(_) | Self::Empty => None,
        }
    }

    /// Returns the wait status of the signal associated with a ptrace-stop,
    /// if it has not yet been reported through `wait`.
    pub(super) fn wait(&mut self, options: WaitOptions) -> Option<PtraceWaitStatus> {
        let this = core::mem::replace(self, Self::Empty);

        match this {
            Self::Pending(signal, wait_status) => {
                if !options.contains(WaitOptions::WNOWAIT) {
                    *self = Self::Consumed(signal);
                } else {
                    *self = Self::Pending(signal, wait_status);
                }
                Some(wait_status)
            }
            Self::Consumed(signal) => {
                *self = Self::Consumed(signal);
                None
            }
            Self::Injected(_) => unreachable!(),
            Self::Empty => None,
        }
    }

    /// Injects a signal by the tracer.
    pub(super) fn inject(&mut self, new_signal: Box<dyn Signal>) {
        let this = core::mem::replace(self, Self::Empty);

        let mut signal = match this {
            Self::Pending(signal, _) | Self::Consumed(signal) => signal,
            Self::Injected(_) | Self::Empty => unreachable!(),
        };

        signal.set_signal(new_signal);
        *self = Self::Injected(signal);
    }

    /// Returns the signal associated with a ptrace-stop,
    /// but does not change the state.
    pub(super) fn get(&self) -> Option<&dyn Signal> {
        match self {
            Self::Pending(signal, _) | Self::Consumed(signal) | Self::Injected(signal) => {
                Some(signal.signal())
            }
            Self::Empty => None,
        }
    }
}

bitflags! {
    /// Options accepted by `PTRACE_SETOPTIONS`.
    pub struct PtraceOptions: usize {
        /// Marks syscall stops with signal number set to `SIGTRAP | 0x80`.
        const PTRACE_O_TRACESYSGOOD = 1;
        /// Stops the tracee at `fork` and automatically traces the new thread.
        const PTRACE_O_TRACEFORK = 1 << PtraceEvent::Fork(0).code();
        /// Stops the tracee at `vfork` and automatically traces the new thread.
        const PTRACE_O_TRACEVFORK = 1 << PtraceEvent::Vfork(0).code();
        /// Stops the tracee at `clone` and automatically traces the new thread.
        const PTRACE_O_TRACECLONE = 1 << PtraceEvent::Clone(0).code();
        /// Stops the tracee at `execve`.
        const PTRACE_O_TRACEEXEC = 1 << PtraceEvent::Exec(0).code();
        /// Stops the tracee at the completion of `vfork`.
        const PTRACE_O_TRACEVFORKDONE = 1 << PtraceEvent::VforkDone(0).code();
        /// Stops the tracee at `exit`.
        const PTRACE_O_TRACEEXIT = 1 << PtraceEvent::Exit(0).code();
        /// Send a `SIGKILL` signal to the tracee if the tracer exits.
        const PTRACE_O_EXITKILL = 1 << 20;
    }
}

/// The events of ptrace-event-stops.
#[derive(Debug, Clone)]
pub enum PtraceEvent {
    /// A `fork` event with the new child thread ID.
    Fork(Tid),
    /// A `vfork` event with the new child thread ID.
    Vfork(Tid),
    /// A `clone` event with the new child thread ID.
    Clone(Tid),
    /// An `execve` event with the former thread ID.
    Exec(Tid),
    /// A done `vfork` event with the child thread ID.
    VforkDone(Tid),
    /// An `exit` event with the tracee's exit code.
    Exit(ExitCode),
}

impl PtraceEvent {
    /// Returns the Linux `PTRACE_EVENT_*` code of this event.
    const fn code(&self) -> u32 {
        match self {
            Self::Fork(_) => 1,
            Self::Vfork(_) => 2,
            Self::Clone(_) => 3,
            Self::Exec(_) => 4,
            Self::VforkDone(_) => 5,
            Self::Exit(_) => 6,
        }
    }

    /// Returns the `PtraceOptions` corresponding to this event.
    pub(super) const fn option(&self) -> PtraceOptions {
        PtraceOptions::from_bits(1 << self.code()).unwrap()
    }

    /// Returns the message of this event.
    pub const fn message(&self) -> usize {
        match self {
            Self::Fork(tid)
            | Self::Vfork(tid)
            | Self::Clone(tid)
            | Self::Exec(tid)
            | Self::VforkDone(tid) => *tid as usize,
            Self::Exit(exit_code) => *exit_code as usize,
        }
    }

    /// Creates a `siginfo_t` for the ptrace-stop triggered by this event.
    pub(super) fn siginfo(&self, ctx: &Context) -> siginfo_t {
        let code = PtraceWaitStatus::from_event(self).0;
        let mut siginfo = siginfo_t::new(SIGTRAP, code);
        siginfo.set_pid_uid_by(ctx);
        siginfo
    }
}

/// The `si_status` code of a ptrace-stop for `wait` syscalls.
#[derive(Copy, Clone)]
pub struct PtraceWaitStatus(i32);

impl PtraceWaitStatus {
    pub(super) fn from_event(event: &PtraceEvent) -> Self {
        Self(SIGTRAP.as_u8() as i32 | ((event.code() as i32) << 8))
    }

    pub(super) fn from_signal(sig: SigNum) -> Self {
        Self(sig.as_u8() as i32)
    }

    pub(super) fn from_syscall(options: &PtraceOptions) -> Self {
        let mut sig = SIGTRAP.as_u8() as i32;
        if options.contains(PtraceOptions::PTRACE_O_TRACESYSGOOD) {
            sig |= 0x80;
        }
        Self(sig)
    }

    pub fn to_wait4_status(self) -> u32 {
        ((self.0 as u32) << 8) | 0x7f
    }

    pub fn to_waitid_si_status(self) -> i32 {
        self.0
    }
}

/// Creates a `siginfo_t` for a syscall-stop.
pub(super) fn syscall_stop_siginfo(options: &PtraceOptions, ctx: &Context) -> siginfo_t {
    let code = PtraceWaitStatus::from_syscall(options).0;
    let mut siginfo = siginfo_t::new(SIGTRAP, code);
    siginfo.set_pid_uid_by(ctx);
    siginfo
}
