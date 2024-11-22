// SPDX-License-Identifier: MPL-2.0

//! This module defines struct `ProcessVm`
//! to represent the layout of user space process virtual memory.
//!
//! The `ProcessVm` struct contains `Vmar`,
//! which stores all existing memory mappings.
//! The `Vm` also contains
//! the basic info of process level vm segments,
//! like init stack and heap.

mod heap;
mod init_stack;

use aster_rights::Full;
pub use heap::Heap;
use ostd::sync::MutexGuard;

pub use self::{
    heap::USER_HEAP_SIZE_LIMIT,
    init_stack::{
        aux_vec::{AuxKey, AuxVec},
        InitStack, InitStackReader, INIT_STACK_SIZE, MAX_ARGV_NUMBER, MAX_ARG_LEN, MAX_ENVP_NUMBER,
        MAX_ENV_LEN,
    },
};
use crate::{
    prelude::*,
    vm::vmar::{Vmar, ROOT_VMAR_GROWUP_BASE},
};

/*
 * The user's virtual memory space layout looks like below.
 * TODO: The layout of the userheap does not match the current implementation,
 * And currently the initial program break is a fixed value.
 *
 *  (high address)
 *  +---------------------+ <------+ The top of Vmar non-allocatable address
 *  |                     |          Randomly padded pages
 *  +---------------------+ <------+ The base of the initial user stack
 *  | User stack          |
 *  |                     |
 *  +---------||----------+ <------+ The user stack limit, can be extended lower
 *  |         \/          |
 *  | ...                 |
 *  |                     |
 *  | MMAP Spaces         |
 *  |                     |
 *  | ...                 |
 *  |         /\          |
 *  +---------||----------+ <------+ The current program break
 *  | User heap           |
 *  |                     |
 *  +---------------------+ <------+ The original program break
 *  |                     |          Randomly padded pages
 *  +---------------------+ <------+ The end of the program's last segment
 *  |                     |
 *  | Loaded segments     |
 *  | .text, .data, .bss  |
 *  | , etc.              |
 *  |                     |
 *  +---------------------+ <------+ The bottom of Vmar at 0x1_0000
 *  |                     |          64 KiB unusable space
 *  +---------------------+
 *  (low address)
 */

/// The process user space virtual memory
pub struct ProcessVm {
    root_vmar: Mutex<Option<Vmar<Full>>>,
    init_stack: InitStack,
    heap: Heap,
}

/// A guard to the [`Vmar`] used by a process.
///
/// It is bound to a [`ProcessVm`] and can only be obtained from
/// the [`ProcessVm::lock_root_vmar`] method.
pub struct ProcessVmarGuard<'a> {
    inner: MutexGuard<'a, Option<Vmar<Full>>>,
}

impl ProcessVmarGuard<'_> {
    /// Gets a reference to the process VMAR.
    pub fn get(&self) -> &Vmar<Full> {
        self.inner.as_ref().unwrap()
    }

    /// Clears the VMAR of the binding process.
    pub(super) fn clear(&mut self) {
        *self.inner = None;
    }
}

impl Clone for ProcessVm {
    fn clone(&self) -> Self {
        let root_vmar = self.lock_root_vmar();
        Self {
            root_vmar: Mutex::new(Some(root_vmar.get().dup().unwrap())),
            init_stack: self.init_stack.fork(),
            heap: self.heap.fork(),
        }
    }
}

impl ProcessVm {
    /// Allocates a new `ProcessVm`
    pub fn alloc() -> Self {
        let root_vmar = Vmar::<Full>::new_root();
        let init_stack = InitStack::new();
        let heap = Heap::new();
        Self {
            root_vmar: Mutex::new(Some(root_vmar)),
            heap,
            init_stack,
        }
    }

    /// Forks a `ProcessVm` from `other`.
    ///
    /// The returned `ProcessVm` will have a forked `Vmar`.
    pub fn fork_from(other: &ProcessVm) -> Result<Self> {
        let process_vmar = other.lock_root_vmar();
        let root_vmar = Mutex::new(Some(Vmar::<Full>::fork_from(process_vmar.get())?));
        Ok(Self {
            root_vmar,
            heap: other.heap.fork(),
            init_stack: other.init_stack.fork(),
        })
    }

    /// Locks the root VMAR and gets a guard to it.
    pub fn lock_root_vmar(&self) -> ProcessVmarGuard {
        ProcessVmarGuard {
            inner: self.root_vmar.lock(),
        }
    }

    /// Returns a reader for reading contents from
    /// the `InitStack`.
    pub fn init_stack_reader(&self) -> InitStackReader {
        self.init_stack.reader(self.lock_root_vmar())
    }

    /// Returns the top address of the user stack.
    pub fn user_stack_top(&self) -> Vaddr {
        self.init_stack.user_stack_top()
    }

    pub(super) fn map_and_write_init_stack(
        &self,
        argv: Vec<CString>,
        envp: Vec<CString>,
        aux_vec: AuxVec,
    ) -> Result<()> {
        let root_vmar = self.lock_root_vmar();
        self.init_stack
            .map_and_write(root_vmar.get(), argv, envp, aux_vec)
    }

    pub(super) fn init_heap(&self, program_break: Vaddr) -> Result<()> {
        debug_assert!(program_break < ROOT_VMAR_GROWUP_BASE);
        let root_vmar = self.lock_root_vmar();
        self.heap.init(root_vmar.get(), program_break)
    }

    pub(super) fn heap(&self) -> &Heap {
        &self.heap
    }

    /// Clears existing mappings and metadata for stack and heap.
    pub(super) fn clear(&self) {
        let root_vmar = self.lock_root_vmar();
        root_vmar.get().clear().unwrap();
        self.heap.clear();
        self.init_stack.clear();
    }
}
