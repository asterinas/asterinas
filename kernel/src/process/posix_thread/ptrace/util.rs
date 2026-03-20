// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::process::ExitCode;

/// The requests that can continue a stopped tracee.
#[expect(dead_code)]
#[derive(Debug)]
pub enum PtraceContRequest {
    Continue,
    SingleStep,
    Syscall,
}

/// The result of a ptrace-stop.
pub enum PtraceStopResult {
    /// The ptrace-stop is continued by the tracer.
    Continued,
    /// The ptrace-stop is interrupted by `SIGKILL`.
    Interrupted,
    /// The thread is not traced.
    NotTraced(Box<dyn Signal>),
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

/// The ptrace-stop events.
#[derive(Debug, Clone)]
pub enum PtraceEvent {
    /// A `fork` event stop with the new child thread ID.
    Fork(Tid),
    /// A `vfork` event stop with the new child thread ID.
    Vfork(Tid),
    /// A `clone` event stop with the new child thread ID.
    Clone(Tid),
    /// An `execve` event stop with the former thread ID.
    Exec(Tid),
    /// A done `vfork` event stop with the child thread ID.
    VforkDone(Tid),
    /// An `exit` event stop with the tracee's exit code.
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
    pub(super) const fn message(&self) -> usize {
        match self {
            Self::Fork(tid)
            | Self::Vfork(tid)
            | Self::Clone(tid)
            | Self::Exec(tid)
            | Self::VforkDone(tid) => *tid as usize,
            Self::Exit(exit_code) => *exit_code as usize,
        }
    }
}

/// The signal info of a ptrace-stop.
#[derive(Default)]
pub(super) enum StopSigInfo {
    /// The signal info that has not yet been waited on.
    UnWaited(siginfo_t),
    /// The signal info that has been waited on.
    Waited(siginfo_t),
    /// No ptrace-stop signal info recorded.
    #[default]
    None,
}

impl StopSigInfo {
    /// Records the signal info of a ptrace-stop.
    pub(super) fn stop(&mut self, siginfo: siginfo_t) {
        *self = Self::UnWaited(siginfo);
    }

    /// Clears the ptrace-stop signal info.
    pub(super) fn clear(&mut self) {
        *self = Self::None;
    }

    /// Waits on the ptrace-stop signal info and returns it,
    /// if it has not yet been waited on.
    pub(super) fn wait(&mut self) -> Option<siginfo_t> {
        match *self {
            Self::UnWaited(siginfo) => {
                *self = Self::Waited(siginfo);
                Some(siginfo)
            }
            Self::Waited(_) | Self::None => None,
        }
    }

    /// Returns the ptrace-stop signal info.
    pub(super) fn get(&self) -> Option<siginfo_t> {
        match self {
            Self::UnWaited(siginfo) | Self::Waited(siginfo) => Some(*siginfo),
            Self::None => None,
        }
    }
}

#[cfg(target_arch = "x86_64")]
macro_rules! general_regs_ptrace_setter {
    ([ $field:ident, $($meta:tt)+ ]) => {
        paste::paste! {
            #[inline(always)]
            pub(super) fn [<ptrace_set_ $field>](regs: &mut GeneralRegs, value: usize) -> Result<()> {
                general_regs_ptrace_setter!(@body regs, value, [ $field, $($meta)+ ]);
                Ok(())
            }
        }
    };

    (@body $regs:ident, $value:ident, [ $field:ident, set ]) => {{
        paste::paste! {
            $regs.[<set_ $field>]($value);
        }
    }};

    (@body $regs:ident, $value:ident, [ $field:ident, set_if($check:expr) ]) => {{
        if ($check)($value) {
            paste::paste! {
                $regs.[<set_ $field>]($value);
            }
        } else {
            return Err(Error::with_message(Errno::EIO, "invalid register value"));
        }
    }};

    (@body $regs:ident, $value:ident, [ $field:ident, set_bits_truncate($mask:expr) ]) => {{
        let old_value = $regs.$field();
        const MASK: usize = $mask;
        paste::paste! {
            $regs.[<set_ $field>]((old_value & !MASK) | ($value & MASK));
        }
    }};

    (@body $regs:ident, $value:ident, [ $field:ident, fixed($expected:expr) ]) => {{
        let _ = $regs;
        const EXPECTED: usize = $expected;
        if $value != EXPECTED {
            return Err(Error::with_message(Errno::EIO, "invalid segment selector"));
        }
    }};
}

#[cfg(target_arch = "x86_64")]
ostd::for_all_general_regs!(general_regs_ptrace_setter);

/// Checks whether the given offset is valid for in `struct user`.
//
// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/arch/x86/include/asm/user_64.h#L103-L132>
#[cfg(target_arch = "x86_64")]
pub(super) fn check_user_offset(offset: usize) -> Result<()> {
    if !offset.is_multiple_of(size_of::<usize>()) {
        return_errno_with_message!(Errno::EIO, "invalid USER area offset");
    }

    // We only support the offsets for general-purpose registers currently.
    // `struct user_regs_struct` is the first field in `struct user`.
    if offset >= size_of::<c_user_regs_struct>() {
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "only offsets for general-purpose registers are supported currently"
        );
    }
    Ok(())
}
