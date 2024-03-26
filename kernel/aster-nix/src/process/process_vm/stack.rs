// SPDX-License-Identifier: MPL-2.0

//! The init stack for the process.
//! The init stack is used to store the `argv` and `envp` and auxiliary vectors.
//! We can read `argv` and `envp` of a process from the init stack.
//! Usually, the lowest address of init stack is
//! the highest address of the user stack of the first thread.
//!
//! However, the init stack will be mapped to user space
//! and the user process can write the content of init stack,
//! so the content reading from init stack may not be the same as the process init status.
//!

use core::sync::atomic::{AtomicUsize, Ordering};

use aster_frame::vm::VmIo;
use aster_rights::Full;

use crate::{
    prelude::*,
    process::constants::{MAX_ARG_LEN, MAX_ENVP_NUMBER, MAX_ENV_LEN},
    vm::vmar::Vmar,
};

/// The init stack for the process.
pub struct Stack {
    vmar: Vmar<Full>,
    base: AtomicUsize,
}

impl Clone for Stack {
    fn clone(&self) -> Self {
        let vmar = self.vmar().dup().unwrap();

        let Ok(base) = self.base() else {
            return Self::new(vmar);
        };

        Self::new_with_base(vmar, base)
    }
}

impl Stack {
    const UNINIT_BASE: Vaddr = 0;

    /// Creates a new init stack.
    /// The stack is uninitialized by default.
    pub(super) const fn new(vmar: Vmar<Full>) -> Self {
        Self {
            vmar,
            base: AtomicUsize::new(Self::UNINIT_BASE),
        }
    }

    const fn new_with_base(vmar: Vmar<Full>, base: Vaddr) -> Self {
        Self {
            vmar,
            base: AtomicUsize::new(base),
        }
    }

    /// Sets the base address of the init stack
    pub(in crate::process) fn set_base(&self, base: Vaddr) {
        self.base.store(base, Ordering::Relaxed)
    }

    /// Returns the lowest address of stack.
    /// If the stack is uninitialized,
    /// this method will return `EINVAL``.
    pub fn base(&self) -> Result<Vaddr> {
        let base = self.base.load(Ordering::Relaxed);
        if base == Self::UNINIT_BASE {
            return_errno_with_message!(Errno::EINVAL, "the stack is uninitialized.");
        }

        Ok(base)
    }

    /// Sets the stack as uninitilized
    pub fn clear(&self) {
        self.base.store(Self::UNINIT_BASE, Ordering::Relaxed);
    }

    fn vmar(&self) -> &Vmar<Full> {
        &self.vmar
    }

    /// Read argc from the process init stack
    pub fn argc(&self) -> Result<u64> {
        let stack_base = self.base()?;
        Ok(self.vmar.read_val(stack_base)?)
    }

    /// Read argv from the process init stack
    pub fn argv(&self) -> Result<Vec<CString>> {
        let argc = self.argc()? as usize;
        // base = stack bottom + the size of argc
        let base = self.base()? + 8;

        let mut argv = Vec::with_capacity(argc);
        for i in 0..argc {
            let arg_ptr = {
                let offset = base + i * 8;
                self.vmar.read_val::<Vaddr>(offset)?
            };

            let arg = read_cstring_from_vmar(&self.vmar, arg_ptr, MAX_ARG_LEN)?;
            argv.push(arg);
        }

        Ok(argv)
    }

    /// Read envp from the process
    pub fn envp(&self) -> Result<Vec<CString>> {
        let argc = self.argc()? as usize;
        // base = stack bottom
        // + the size of argc(8)
        // + the size of arg pointer(8) * the number of arg(argc)
        // + the size of null pointer(8)
        let base = self.base()? + 8 + 8 * argc + 8;

        let mut envp = Vec::new();
        for i in 0..MAX_ENVP_NUMBER {
            let envp_ptr = {
                let offset = base + i * 8;
                self.vmar.read_val::<Vaddr>(offset)?
            };

            if envp_ptr == 0 {
                break;
            }

            let env = read_cstring_from_vmar(&self.vmar, envp_ptr, MAX_ENV_LEN)?;
            envp.push(env);
        }

        Ok(envp)
    }
}

// TODO: use the algorithm introduced by PR #623
fn read_cstring_from_vmar(vmar: &Vmar<Full>, addr: Vaddr, max_len: usize) -> Result<CString> {
    let mut buffer = vec![0u8; max_len];
    vmar.read_bytes(addr, &mut buffer)?;
    Ok(CString::from(CStr::from_bytes_until_nul(&buffer)?))
}
