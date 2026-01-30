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
    heap::{Heap, LockedHeap},
    init_stack::{
        INIT_STACK_SIZE, InitStack, InitStackReader, MAX_LEN_STRING_ARG, MAX_NR_STRING_ARGS,
        aux_vec::{AuxKey, AuxVec},
    },
};
use crate::{fs::path::Path, prelude::*, vm::vmar::Vmar};

/*
 * The user's virtual memory space layout looks like below.
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
    /// The executable file.
    executable_file: Path,
    /// The base address for vDSO segment
    #[cfg(target_arch = "riscv64")]
    vdso_base: AtomicUsize,
}

impl ProcessVm {
    /// Creates a new `ProcessVm` without mapping anything.
    pub(super) fn new(executable_file: Path) -> Self {
        Self {
            init_stack: InitStack::new(),
            heap: Heap::new_uninitialized(),
            executable_file,
            #[cfg(target_arch = "riscv64")]
            vdso_base: AtomicUsize::new(0),
        }
    }

    /// Creates a new `ProcessVm` with identical contents of an existing one.
    pub fn fork_from(process_vm: &Self, heap_guard: &LockedHeap) -> Self {
        Self {
            init_stack: process_vm.init_stack.clone(),
            heap: Heap::fork_from(heap_guard),
            executable_file: process_vm.executable_file.clone(),
            #[cfg(target_arch = "riscv64")]
            vdso_base: AtomicUsize::new(process_vm.vdso_base.load(Ordering::Relaxed)),
        }
    }

    /// Returns the initial portion of the main stack of a process.
    pub fn init_stack(&self) -> &InitStack {
        &self.init_stack
    }

    /// Returns the user heap.
    pub fn heap(&self) -> &Heap {
        &self.heap
    }

    /// Returns a reference to the executable `Path`.
    pub fn executable_file(&self) -> &Path {
        &self.executable_file
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

    /// Maps and initializes the heap virtual memory.
    pub(super) fn map_and_init_heap(
        &self,
        vmar: &Vmar,
        data_segment_size: usize,
        heap_base: Vaddr,
    ) -> Result<()> {
        self.heap()
            .map_and_init_heap(vmar, data_segment_size, heap_base)
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
    pub fn init_stack_reader(&self) -> Option<InitStackReader<'_>> {
        self.as_ref()
            .map(|vmar| vmar.process_vm().init_stack.reader(vmar))
    }
}

/// Activates the [`Vmar`] in the current process's context.
pub(super) fn activate_vmar(ctx: &Context, new_vmar: Arc<Vmar>) {
    let mut vmar_guard = ctx.process.lock_vmar();
    // Disable preemption because `thread_local::vmar()` will be borrowed during a context switch.
    let _preempt_guard = disable_preempt();

    *ctx.thread_local.vmar().borrow_mut() = Some(new_vmar.clone());
    new_vmar.vm_space().activate();

    vmar_guard.set_vmar(Some(new_vmar));
}
