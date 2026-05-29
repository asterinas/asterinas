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

use core::ops::Range;
#[cfg(target_arch = "riscv64")]
use core::sync::atomic::{AtomicUsize, Ordering};

use ostd::task::disable_preempt;

pub use self::{
    heap::{Heap, LockedHeap},
    init_stack::{
        INIT_STACK_SIZE, InitStack, InitStackReader, MAX_LEN_STRING_ARG, MAX_NR_STRING_ARGS,
        aux_vec::{AuxKey, AuxVec},
    },
};
use crate::{
    fs::vfs::path::Path,
    prelude::*,
    vm::vmar::{Vmar, VmarHandle},
};

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
    /// The code range from the executable file.
    code_range: SpinLock<Range<Vaddr>>,
    /// The data range from the executable file.
    data_range: SpinLock<Range<Vaddr>>,
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
            code_range: SpinLock::new(0..0),
            data_range: SpinLock::new(0..0),
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
            code_range: SpinLock::new(process_vm.code_range.lock().clone()),
            data_range: SpinLock::new(process_vm.data_range.lock().clone()),
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

    /// Returns the code range from the executable file.
    pub fn code_range(&self) -> Range<Vaddr> {
        self.code_range.lock().clone()
    }

    /// Returns the data range from the executable file.
    pub fn data_range(&self) -> Range<Vaddr> {
        self.data_range.lock().clone()
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

    /// Updates the code range from the executable file.
    pub(super) fn set_code_range(&self, range: Range<Vaddr>) {
        *self.code_range.lock() = range;
    }

    /// Updates the data range from the executable file.
    pub(super) fn set_data_range(&self, range: Range<Vaddr>) {
        *self.data_range.lock() = range;
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

/// A snapshot of the process VMAR identity.
///
/// This type is used only for identity comparison.
#[derive(Clone, Debug)]
pub struct VmarSnapshot(Weak<Vmar>);

impl VmarSnapshot {
    /// Returns the raw identity pointer of the captured `Vmar`.
    pub fn as_ptr(&self) -> *const Vmar {
        Weak::as_ptr(&self.0)
    }

    /// Returns whether two snapshots refer to the same `Vmar`.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Weak::ptr_eq(&self.0, &other.0)
    }
}

impl From<Weak<Vmar>> for VmarSnapshot {
    fn from(snapshot: Weak<Vmar>) -> Self {
        Self(snapshot)
    }
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

    /// Takes a snapshot of the current VMAR identity.
    pub fn snapshot(&self) -> VmarSnapshot {
        VmarSnapshot(self.inner.as_ref().map(Arc::downgrade).unwrap_or_default())
    }

    /// Returns whether the current VMAR has the same identity as the `snapshot`.
    pub fn is_same_as(&self, snapshot: &VmarSnapshot) -> bool {
        self.inner
            .as_ref()
            .is_some_and(|vmar| core::ptr::eq(Arc::as_ptr(vmar), Weak::as_ptr(&snapshot.0)))
    }

    /// Sets a new VMAR for the binding process.
    ///
    /// This method will return the old VMAR.
    ///
    /// If the `new_vmar` is `None`, this method will remove the
    /// current VMAR.
    pub(super) fn set_vmar(&mut self, new_vmar: Option<Arc<Vmar>>) -> Option<Arc<Vmar>> {
        core::mem::replace(&mut *self.inner, new_vmar)
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
///
/// Returns a [`ProcessVmarGuard`] that keeps the process VMAR lock held and the old [`Vmar`].
pub(super) fn activate_vmar<'a>(
    ctx: &'a Context<'a>,
    new_vmar: VmarHandle,
) -> (ProcessVmarGuard<'a>, VmarHandle) {
    let vmar_arc = new_vmar.clone_arc();

    let mut vmar_guard = ctx.process.lock_vmar();

    // Disable preemption because `thread_local::vmar()` will be borrowed during a context switch.
    let old_vmar = {
        let _preempt_guard = disable_preempt();
        let old_vmar = ctx
            .thread_local
            .vmar()
            .borrow_mut()
            .replace(new_vmar)
            .unwrap();
        vmar_arc.vm_space().activate();
        old_vmar
    };
    vmar_guard.set_vmar(Some(vmar_arc));

    (vmar_guard, old_vmar)
}
