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
pub use heap::ProgramBreak;
use ostd::{sync::MutexGuard, task::disable_preempt};

pub use self::{
    heap::USER_HEAP_SIZE_LIMIT,
    init_stack::{
        aux_vec::{AuxKey, AuxVec},
        InitStack, InitStackReader, INIT_STACK_SIZE, MAX_LEN_STRING_ARG, MAX_NR_STRING_ARGS,
    },
};
use crate::{prelude::*, vm::vmar::Vmar};

/*
 * The user's virtual memory space layout looks like below.
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
 *  +---------------------+ <------+ The end of the program's last data segment,
 *  |                     |          also the initial program break
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
    heap: ProgramBreak,
}

/// A guard to the [`Vmar`] used by a process.
///
/// It is bound to a [`ProcessVm`] and can only be obtained from
/// the [`ProcessVm::lock_root_vmar`] method.
pub struct ProcessVmarGuard<'a> {
    inner: MutexGuard<'a, Option<Vmar<Full>>>,
}

impl ProcessVmarGuard<'_> {
    /// Unwraps and returns a reference to the process VMAR.
    ///
    /// # Panics
    ///
    /// This method will panic if the process has exited and its VMAR has been dropped.
    pub fn unwrap(&self) -> &Vmar<Full> {
        self.inner.as_ref().unwrap()
    }

    /// Returns a reference to the process VMAR if it exists.
    ///
    /// Returns `None` if the process has exited and its VMAR has been dropped.
    pub fn as_ref(&self) -> Option<&Vmar<Full>> {
        self.inner.as_ref()
    }

    /// Sets a new VMAR for the binding process.
    ///
    /// If the `new_vmar` is `None`, this method will remove the
    /// current VMAR.
    pub(super) fn set_vmar(&mut self, new_vmar: Option<Vmar<Full>>) {
        *self.inner = new_vmar;
    }
}

impl Clone for ProcessVm {
    fn clone(&self) -> Self {
        let root_vmar = self.lock_root_vmar();
        Self {
            root_vmar: Mutex::new(Some(root_vmar.unwrap().dup().unwrap())),
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
        let heap = ProgramBreak::new_uninit();
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
        let root_vmar = Mutex::new(Some(Vmar::<Full>::fork_from(process_vmar.unwrap())?));
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
            .map_and_write(root_vmar.unwrap(), argv, envp, aux_vec)
    }

    pub(super) fn init_heap_and_map_clearance(&self, program_break: Vaddr) -> Result<()> {
        let root_vmar = self.lock_root_vmar();
        self.heap
            .init_and_map_clearance(root_vmar.unwrap(), program_break)
    }

    pub(super) fn heap(&self) -> &ProgramBreak {
        &self.heap
    }

    /// Clears existing mappings, including the heap and init stack.
    pub fn clear(&self) {
        let root_vmar = self.lock_root_vmar();
        root_vmar.unwrap().clear().unwrap();
        self.heap.clear();
        self.init_stack.clear();
    }
}

/// Set the [`ProcessVm`] of the current process as a new empty `Vmar`.
///
/// The stack and the heap are also cleared.
pub fn renew_vm(ctx: &Context) {
    let process_vm = ctx.process.vm();
    let mut root_vmar = process_vm.lock_root_vmar();

    let new_vmar = Vmar::<Full>::new_root();
    let guard = disable_preempt();
    *ctx.thread_local.root_vmar().borrow_mut() = Some(new_vmar.dup().unwrap());
    new_vmar.vm_space().activate();
    root_vmar.set_vmar(Some(new_vmar));

    process_vm.heap.clear();
    process_vm.init_stack.clear();

    drop(guard);
}
