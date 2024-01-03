//! This module defines struct `Vm` to represent the layout of user space process virtual memory.
//!
//! The `Vm` struct contains `Vmar`, which stores all existing memory mappings. The `Vm` also contains
//! the basic info of process level vm segments, like init stack and heap.
//!
//! TODO: The vm layout should be redesigned in the future. Currently, we only use a small part of the
//! whole virtual memory space.
//!

mod heap;
mod stack;

use super::constants::{MAX_ARG_LEN, MAX_ENVP_NUMBER, MAX_ENV_LEN};
use crate::prelude::*;
use crate::vm::vmar::Vmar;
use alloc::ffi::CString;
use aster_frame::vm::VmIo;
use aster_rights::Full;

pub use heap::Heap;
pub use stack::Stack;

/*
* The user space virtual memory layout looks like below.
* |-----------------------|-------The highest user vm address
* |                       |
* |       Mmap Areas      |
* |                       |
* |                       |
* --------------------------------The init stack base
* |                       |
* | User Stack(Init Stack)|
* |                       |
* |         ||            |
* ----------||----------------------The user stack top, grows down
* |         \/            |
* |                       |
* |     Unmapped Areas    |
* |                       |
* |         /\            |
* ----------||---------------------The user heap top, grows up
* |         ||            |
* |                       |
* |        User Heap      |
* |                       |
* ----------------------------------The user heap base
*/

/// The process user space virtual memory
pub struct Vm {
    root_vmar: Vmar<Full>,
    heap: Heap,
    stack: Stack,
}

impl Clone for Vm {
    fn clone(&self) -> Self {
        Self {
            root_vmar: self.root_vmar.dup().unwrap(),
            heap: self.heap.clone(),
            stack: self.stack.clone(),
        }
    }
}

impl Vm {
    pub fn alloc() -> Self {
        let root_vmar = Vmar::<Full>::new_root();
        let heap = Heap::new();
        let stack = Stack::new();
        heap.init(&root_vmar);
        Vm {
            root_vmar,
            heap,
            stack,
        }
    }

    pub fn new(root_vmar: Vmar<Full>, heap: Heap, stack: Stack) -> Self {
        Self {
            root_vmar,
            heap,
            stack,
        }
    }

    pub fn root_vmar(&self) -> &Vmar<Full> {
        &self.root_vmar
    }

    pub(super) fn stack(&self) -> &Stack {
        &self.stack
    }

    pub fn heap(&self) -> &Heap {
        &self.heap
    }

    /// Set the `Vm` to the init status
    pub fn clear(&self) {
        self.root_vmar.clear().unwrap();
        self.heap.set_default(&self.root_vmar);
        self.stack.clear();
    }

    /// Read argc from the process init stack
    pub fn argc(&self) -> Result<u64> {
        let stack_bottom = self.stack.bottom()?;
        Ok(self.root_vmar.read_val(stack_bottom)?)
    }

    /// Read argv from the process init stack
    pub fn argv(&self) -> Result<Vec<CString>> {
        let argc = self.argc()? as usize;
        // base = stack bottom + the size of argc
        let base = self.stack.bottom()? + 8;

        let mut argv = Vec::with_capacity(argc);
        for i in 0..argc {
            let arg_ptr = {
                let offset = base + i * 8;
                self.root_vmar.read_val::<Vaddr>(offset)?
            };

            let arg = read_cstring_from_vmar(&self.root_vmar, arg_ptr, MAX_ARG_LEN)?;
            argv.push(arg);
        }

        Ok(argv)
    }

    /// Read envp from the process
    pub fn envp(&self) -> Result<Vec<CString>> {
        let argc = self.argc()? as usize;
        // base = stack bottom + the size of argc(8) + the size of arg pointer(8) * the number of arg +
        // the size of null pointer(8)
        let base = self.stack.bottom()? + 8 + 8 * argc + 8;

        let mut envp = Vec::new();
        for i in 0..MAX_ENVP_NUMBER {
            let envp_ptr = {
                let offset = base + i * 8;
                self.root_vmar.read_val::<Vaddr>(offset)?
            };

            if envp_ptr == 0 {
                break;
            }

            let env = read_cstring_from_vmar(&self.root_vmar, envp_ptr, MAX_ENV_LEN)?;
            envp.push(env);
        }

        Ok(envp)
    }
}

fn read_cstring_from_vmar(vmar: &Vmar<Full>, addr: Vaddr, max_len: usize) -> Result<CString> {
    let mut buffer = vec![0u8; max_len];
    vmar.read_bytes(addr, &mut buffer)?;
    Ok(CString::from(CStr::from_bytes_until_nul(&buffer)?))
}
