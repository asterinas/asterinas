// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

/// User-provided signal stack. `SigStack` is per-thread, and each thread can have
/// at most one `SigStack`. If one signal handler specifying the `SA_ONSTACK` flag,
/// the handler should be executed on the `SigStack`, instead of on the default stack.
///
/// SigStack can be registered and unregistered by syscall `sigaltstack`.
#[derive(Debug, Clone)]
pub struct SigStack {
    base: Vaddr,
    flags: SigStackFlags,
    size: usize,
    /// The number of handlers that are currently using the stack
    handler_counter: usize,
}

bitflags! {
    pub struct SigStackFlags: u32 {
        const SS_ONSTACK = 1 << 0;
        const SS_DISABLE = 1 << 1;
        const SS_AUTODISARM = 1 << 31;
    }
}

#[repr(u8)]
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SigStackStatus {
    #[default]
    SS_INACTIVE = 0,
    // The thread is currently executing on the alternate signal stack
    SS_ONSTACK = 1,
    // The stack is currently disabled.
    SS_DISABLE = 2,
}

impl SigStack {
    pub fn new(base: Vaddr, flags: SigStackFlags, size: usize) -> Self {
        Self {
            base,
            flags,
            size,
            handler_counter: 0,
        }
    }

    pub fn base(&self) -> Vaddr {
        self.base
    }

    pub fn flags(&self) -> SigStackFlags {
        self.flags
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn status(&self) -> SigStackStatus {
        // Learning From [sigaltstack doc](https://man7.org/linux/man-pages/man2/sigaltstack.2.html):
        // If the stack is currently executed on,
        // 1. If the stack was established with flag SS_AUTODISARM, the stack status is DISABLE,
        // 2. otherwise, the stack status is ONSTACK
        if self.handler_counter == 0 {
            if self.flags.contains(SigStackFlags::SS_AUTODISARM) {
                SigStackStatus::SS_DISABLE
            } else {
                SigStackStatus::SS_INACTIVE
            }
        } else {
            SigStackStatus::SS_ONSTACK
        }
    }

    /// Mark the stack is currently used by a signal handler.    
    pub fn increase_handler_counter(&mut self) {
        self.handler_counter += 1;
    }

    // Mark the stack is freed by current handler.
    pub fn decrease_handler_counter(&mut self) {
        // FIXME: deal with SS_AUTODISARM flag
        self.handler_counter -= 1
    }

    /// Determines whether the stack is executed on by any signal handler
    pub fn is_active(&self) -> bool {
        (self.handler_counter > 0)
            && !(self.flags.intersects(SigStackFlags::SS_AUTODISARM)
                || self.flags.intersects(SigStackFlags::SS_DISABLE))
    }

    pub fn is_disabled(&self) -> bool {
        self.flags.contains(SigStackFlags::SS_DISABLE)
            || (self.handler_counter > 0 && self.flags.contains(SigStackFlags::SS_AUTODISARM))
    }
}
