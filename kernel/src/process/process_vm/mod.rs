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

use ostd::{sync::MutexGuard, task::disable_preempt};

pub use self::{
    heap::{Heap, USER_HEAP_SIZE_LIMIT},
    init_stack::{
        aux_vec::{AuxKey, AuxVec},
        InitStack, InitStackReader, INIT_STACK_SIZE, MAX_LEN_STRING_ARG, MAX_NR_STRING_ARGS,
    },
};
use crate::{prelude::*, vm::vmar::Vmar};

/*
 * The user's virtual memory space layout looks like below.
 * TODO: The layout of the userheap does not match the current implementation,
 * And currently the initial program break is a fixed value.
 *
 *  (high address)
 *  +---------------------+ <------+ The top of Vmar, which is the highest address usable
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
pub struct ProcessVm(Mutex<Option<Arc<Vmar>>>);

/// A guard to the [`Vmar`] used by a process.
///
/// It is bound to a [`ProcessVm`] and can only be obtained from
/// the [`ProcessVm::lock_vmar`] method.
pub struct ProcessVmarGuard<'a> {
    inner: MutexGuard<'a, Option<Arc<Vmar>>>,
}

impl ProcessVmarGuard<'_> {
    /// Unwraps and returns a reference to the process VMAR.
    ///
    /// # Panics
    ///
    /// This method will panic if the process has exited and its VMAR has been dropped.
    pub fn unwrap(&self) -> &Vmar {
        self.inner.as_ref().unwrap()
    }

    /// Returns a reference to the process VMAR if it exists.
    ///
    /// Returns `None` if the process has exited and its VMAR has been dropped.
    pub fn as_ref(&self) -> Option<&Vmar> {
        self.inner.as_ref().map(|v| &**v)
    }

    /// Sets a new VMAR for the binding process.
    ///
    /// If the `new_vmar` is `None`, this method will remove the
    /// current VMAR.
    pub(super) fn set_vmar(&mut self, new_vmar: Option<Arc<Vmar>>) {
        *self.inner = new_vmar;
    }

    /// Duplicates a new VMAR from the binding process.
    ///
    /// This method should only be used when creating a process that
    /// shares the same VMAR.
    pub(super) fn dup_vmar(&self) -> Option<Arc<Vmar>> {
        self.inner.as_ref().cloned()
    }
}

impl Clone for ProcessVm {
    fn clone(&self) -> Self {
        let vmar = self.lock_vmar().dup_vmar();
        Self(Mutex::new(vmar))
    }
}

impl ProcessVm {
    /// Allocates a new `ProcessVm`
    pub fn alloc() -> Self {
        let vmar = Vmar::new();
        let heap = vmar.heap();
        heap.alloc_and_map(&vmar).unwrap();
        Self(Mutex::new(Some(vmar)))
    }

    /// Forks a `ProcessVm` from `other`.
    ///
    /// The returned `ProcessVm` will have a forked `Vmar`.
    pub fn fork_from(other: &ProcessVm) -> Result<Self> {
        let process_vmar = other.lock_vmar();
        let vmar = Mutex::new(Some(Vmar::fork_from(process_vmar.unwrap())?));
        Ok(Self(vmar))
    }

    /// Locks the VMAR and gets a guard to it.
    pub fn lock_vmar(&self) -> ProcessVmarGuard {
        ProcessVmarGuard {
            inner: self.0.lock(),
        }
    }

    /// Clears existing mappings and then maps the heap VMO to the current VMAR.
    pub fn clear_and_map_heap(&self) {
        let vmar = self.lock_vmar();
        let vmar = vmar.unwrap();
        vmar.clear();
        vmar.heap().alloc_and_map(vmar).unwrap();
    }
}

// TODO: Move the below code to the vm module.
impl Vmar {
    /// Returns a reader for reading contents from
    /// the `InitStack`.
    pub fn init_stack_reader(&self) -> InitStackReader {
        self.init_stack().reader(self)
    }

    pub(super) fn map_and_write_init_stack(
        &self,
        argv: Vec<CString>,
        envp: Vec<CString>,
        aux_vec: AuxVec,
    ) -> Result<()> {
        self.init_stack().map_and_write(self, argv, envp, aux_vec)
    }
}

/// Unshares and renews the [`Vmar`] of the current process.
pub(super) fn unshare_and_renew_vmar(ctx: &Context, vmar: &mut ProcessVmarGuard) {
    let new_vmar = Vmar::new();
    let guard = disable_preempt();
    *ctx.thread_local.vmar().borrow_mut() = Some(new_vmar.clone());
    new_vmar.vm_space().activate();
    vmar.set_vmar(Some(new_vmar));
    drop(guard);

    let new_vmar = vmar.unwrap();
    new_vmar.heap().alloc_and_map(new_vmar).unwrap();
}
