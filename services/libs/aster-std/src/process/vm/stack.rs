//! The init stack for the process. The init stack is used to store the `argv` and `envp` and auxiliary
//! vectors. We can read `argv` and `envp` of a process from the init stack. Usually, the bottom address
//! of init stack is the top address of the user stack of the first thread.
//!
//! However, the init stack will be mapped to user space and the user process can write the content of init
//! stack, so the content reading from init stack may not be the same as the process init status.
//!

use crate::prelude::*;
use core::sync::atomic::{AtomicUsize, Ordering};

pub struct Stack {
    bottom: AtomicUsize,
}

impl Clone for Stack {
    fn clone(&self) -> Self {
        let Ok(bottom) = self.bottom() else {
            return Self::new();
        };

        Self::new_with_bottom(bottom)
    }
}

impl Stack {
    const UNINIT: Vaddr = 0;

    pub const fn new() -> Self {
        Self {
            bottom: AtomicUsize::new(Self::UNINIT),
        }
    }

    const fn new_with_bottom(bottom: Vaddr) -> Self {
        Self {
            bottom: AtomicUsize::new(bottom),
        }
    }

    pub fn set(&self, bottom: Vaddr) {
        self.bottom.store(bottom, Ordering::Relaxed)
    }

    /// Returns the bottom address of stack. If the stack is uninitialized, this method
    /// will return error.
    pub fn bottom(&self) -> Result<Vaddr> {
        let bottom = self.bottom.load(Ordering::Relaxed);
        if bottom == Self::UNINIT {
            return_errno_with_message!(Errno::EINVAL, "the stack is uninitialized.");
        }

        Ok(bottom)
    }

    /// Set to uninit value
    pub fn clear(&self) {
        self.bottom.store(Self::UNINIT, Ordering::Relaxed);
    }
}

impl Default for Stack {
    fn default() -> Self {
        Self::new()
    }
}
