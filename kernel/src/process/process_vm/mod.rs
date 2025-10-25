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

#[cfg(target_arch = "riscv64")]
use core::sync::atomic::{AtomicUsize, Ordering};

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
pub struct ProcessVm {
    /// The initial portion of the main stack of a process.
    init_stack: InitStack,
    /// The user heap
    heap: Heap,
    /// The base address for vDSO segment
    #[cfg(target_arch = "riscv64")]
    vdso_base: AtomicUsize,
}

impl ProcessVm {
    /// Creates a new `ProcessVm` without mapping anything.
    pub fn new() -> Self {
        Self {
            init_stack: InitStack::new(),
            heap: Heap::new(),
            #[cfg(target_arch = "riscv64")]
            vdso_base: AtomicUsize::new(0),
        }
    }

    /// Creates a new `ProcessVm` with identical contents of an existing one.
    pub fn fork_from(process_vm: &Self) -> Self {
        Self {
            init_stack: process_vm.init_stack.clone(),
            heap: process_vm.heap.clone(),
            #[cfg(target_arch = "riscv64")]
            vdso_base: AtomicUsize::new(process_vm.vdso_base.load(Ordering::Relaxed)),
        }
    }

    /// Returns the initial portion of the main stack of a process.
    pub(super) fn init_stack(&self) -> &InitStack {
        &self.init_stack
    }

    /// Returns the user heap.
    pub fn heap(&self) -> &Heap {
        &self.heap
    }

    /// Maps and writes the initial portion of the main stack of a process.
    pub(super) fn map_and_write_init_stack(
        &self,
        vmar: &Vmar,
        argv: Vec<CString>,
        envp: Vec<CString>,
        aux_vec: AuxVec,
    ) -> Result<()> {
        self.init_stack().map_and_write(vmar, argv, envp, aux_vec)
    }

    /// Returns the base address for vDSO segment.
    #[cfg(target_arch = "riscv64")]
    pub(super) fn vdso_base(&self) -> Vaddr {
        self.vdso_base.load(Ordering::Relaxed)
    }

    /// Sets the base address for vDSO segment.
    #[cfg(target_arch = "riscv64")]
    pub(super) fn set_vdso_base(&self, addr: Vaddr) {
        self.vdso_base.store(addr, Ordering::Relaxed);
    }
}

/// A guard to the [`Vmar`] used by a process.
///
/// It is bound to a [`Process`] and can only be obtained from
/// the [`Process::lock_vmar`] method.
///
/// [`Process`]: super::process::Process
/// [`Process::lock_vmar`]: super::process::Process::lock_vmar
pub struct ProcessVmarGuard<'a> {
    inner: MutexGuard<'a, Option<Arc<Vmar>>>,
}

impl<'a> ProcessVmarGuard<'a> {
    /// Creates a new VMAR guard from the mutex guard.
    ///
    /// This method should only used by [`Process::lock_vmar`].
    ///
    /// [`Process::lock_vmar`]: super::process::Process::lock_vmar
    pub(super) fn new(inner: MutexGuard<'a, Option<Arc<Vmar>>>) -> Self {
        Self { inner }
    }

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
    /// This method should only be used to clone the VMAR in the `Process`
    /// and store it in the `ThreadLocal`.
    pub(super) fn dup_vmar(&self) -> Option<Arc<Vmar>> {
        self.inner.as_ref().cloned()
    }

    /// Returns a reader for reading contents from
    /// the initial portion of the main stack of a process.
    ///
    /// Returns `None` if the process has exited and its VMAR has been dropped.
    pub fn init_stack_reader(&self) -> Option<InitStackReader> {
        self.as_ref()
            .map(|vmar| vmar.process_vm().init_stack.reader(vmar))
    }
}

/// Creates a new VMAR and map the heap.
///
/// This method should only be used to create a VMAR for the init process.
pub(super) fn new_vmar_and_map() -> Arc<Vmar> {
    let new_vmar = Vmar::new();
    new_vmar
        .process_vm()
        .heap()
        .alloc_and_map(new_vmar.as_ref())
        .unwrap();
    new_vmar
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
    new_vmar
        .process_vm()
        .heap()
        .alloc_and_map(new_vmar)
        .unwrap();
}
