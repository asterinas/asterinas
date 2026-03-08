// SPDX-License-Identifier: MPL-2.0

use super::*;

/// The requests that can continue a stopped tracee.
#[derive(Debug)]
#[expect(dead_code)]
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
    #[expect(dead_code)]
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
