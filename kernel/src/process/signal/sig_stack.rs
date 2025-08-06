// SPDX-License-Identifier: MPL-2.0

use crate::{prelude::*, process::signal::c_types::stack_t};

/// User-provided signal stack.
///
/// Signal stack is per-thread, and each thread can have at most one signal stack.
/// If one signal handler specifying the `SA_ONSTACK` flag,
/// the handler should be executed on the signal stack, instead of on the default stack.
///
/// Signal stack can be registered and unregistered by syscall `sigaltstack`.
#[derive(Debug, Default)]
pub struct SigStack {
    base: Vaddr,
    flags: SigStackFlags,
    size: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SigStackStatus {
    /// The stack is enabled but currently inactive.
    Inactive,
    /// The stack is currently active.
    Active,
    /// The stack is disabled.
    Disable,
}

bitflags! {
    #[derive(Default)]
    pub struct SigStackFlags: u32 {
        const SS_ONSTACK = 1 << 0;
        const SS_DISABLE = 1 << 1;
        const SS_AUTODISARM = 1 << 31;
    }
}

impl SigStack {
    /// Creates a new signal stack.
    pub fn new(base: Vaddr, flags: SigStackFlags, size: usize) -> Self {
        Self { base, flags, size }
    }

    /// Returns the lowest address of the signal stack.
    pub fn base(&self) -> Vaddr {
        self.base
    }

    /// Returns the signal stack flags as set by the user.
    pub fn flags(&self) -> SigStackFlags {
        self.flags
    }

    /// Returns the current active status of the signal stack
    /// based on the given stack pointer.
    pub fn active_status(&self, sp: usize) -> SigStackStatus {
        if self.size == 0 {
            return SigStackStatus::Disable;
        }

        if self.contains(sp) {
            return SigStackStatus::Active;
        }

        SigStackStatus::Inactive
    }

    /// Returns the signal stack size.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Returns whether the given stack pointer is currently on the alternate signal stack.
    ///
    /// Note that if the `SS_AUTODISARM` flag is set,
    /// the alternate signal stack is automatically disarmed after use.
    /// In this case, even if `sp` lies within the stack range,
    /// we consider that the signal stack is not active.
    pub fn contains(&self, sp: usize) -> bool {
        if self.flags().contains(SigStackFlags::SS_AUTODISARM) {
            return false;
        }

        // The stack grows down, so `self.base` is exclusive.
        self.base < sp && sp <= self.base + self.size
    }

    /// Resets the signal stack settings.
    pub(super) fn reset(&mut self) {
        self.base = 0;
        self.size = 0;
        self.flags = SigStackFlags::SS_DISABLE;
    }
}

impl From<&SigStack> for stack_t {
    fn from(value: &SigStack) -> Self {
        Self {
            ss_sp: value.base,
            ss_flags: value.flags.bits as _,
            ss_size: value.size,
        }
    }
}
