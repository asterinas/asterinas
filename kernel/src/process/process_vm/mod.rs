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

use align_ext::AlignExt;
use aster_rights::Full;
pub use heap::Heap;
use ostd::{
    mm::{io_util::HasVmReaderWriter, vm_space::VmQueriedItem, PageFlags, UFrame},
    sync::MutexGuard,
    task::disable_preempt,
};

pub use self::{
    heap::USER_HEAP_SIZE_LIMIT,
    init_stack::{
        aux_vec::{AuxKey, AuxVec},
        InitStack, InitStackReader, INIT_STACK_SIZE, MAX_LEN_STRING_ARG, MAX_NR_STRING_ARGS,
    },
};
use crate::{
    prelude::*,
    thread::exception::PageFaultInfo,
    vm::{
        page_fault_handler::PageFaultHandler,
        vmar::{is_userspace_vaddr, Vmar},
    },
};

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
    root_vmar: Mutex<Option<Vmar<Full>>>,
    init_stack: InitStack,
    heap: Heap,
    #[cfg(target_arch = "riscv64")]
    vdso_base: AtomicUsize,
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
            init_stack: self.init_stack.clone(),
            heap: self.heap.clone(),
            #[cfg(target_arch = "riscv64")]
            vdso_base: AtomicUsize::new(self.vdso_base.load(Ordering::Relaxed)),
        }
    }
}

impl ProcessVm {
    /// Allocates a new `ProcessVm`
    pub fn alloc() -> Self {
        let root_vmar = Vmar::<Full>::new_root();
        let init_stack = InitStack::new();
        let heap = Heap::new();
        heap.alloc_and_map_vm(&root_vmar).unwrap();
        Self {
            root_vmar: Mutex::new(Some(root_vmar)),
            heap,
            init_stack,
            #[cfg(target_arch = "riscv64")]
            vdso_base: AtomicUsize::new(0),
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
            heap: other.heap.clone(),
            init_stack: other.init_stack.clone(),
            #[cfg(target_arch = "riscv64")]
            vdso_base: AtomicUsize::new(other.vdso_base.load(Ordering::Relaxed)),
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
        let root_vmar: ProcessVmarGuard<'_> = self.lock_root_vmar();
        self.init_stack
            .map_and_write(root_vmar.unwrap(), argv, envp, aux_vec)
    }

    pub(super) fn heap(&self) -> &Heap {
        &self.heap
    }

    #[cfg(target_arch = "riscv64")]
    pub(super) fn vdso_base(&self) -> Vaddr {
        self.vdso_base.load(Ordering::Relaxed)
    }

    #[cfg(target_arch = "riscv64")]
    pub(super) fn set_vdso_base(&self, addr: Vaddr) {
        self.vdso_base.store(addr, Ordering::Relaxed);
    }

    /// Clears existing mappings and then maps the heap VMO to the current VMAR.
    pub fn clear_and_map(&self) {
        let root_vmar = self.lock_root_vmar();
        root_vmar.unwrap().clear().unwrap();
        self.heap.alloc_and_map_vm(root_vmar.unwrap()).unwrap();
    }
}

impl ProcessVm {
    /// Reads memory from the process user space.
    ///
    /// The corresponding `VmSpace` is not required be activated on the current CPU.
    pub fn read_remote(&self, vaddr: Vaddr, writer: &mut VmWriter) -> Result<usize> {
        let len = writer.avail();
        let read = |frame: UFrame, skip_offset: usize| {
            let mut reader = frame.reader();
            reader.skip(skip_offset);
            reader.read_fallible(writer)
        };

        self.access_remote(vaddr, len, PageFlags::R, read)
    }

    /// Writes memory to the process user space.
    ///
    /// The corresponding `VmSpace` is not required be activated on the current CPU.
    pub fn write_remote(&self, vaddr: Vaddr, reader: &mut VmReader) -> Result<usize> {
        let len = reader.remain();
        let write = |frame: UFrame, skip_offset: usize| {
            let mut writer = frame.writer();
            writer.skip(skip_offset);
            writer.write_fallible(reader)
        };

        self.access_remote(vaddr, len, PageFlags::W, write)
    }

    /// Access memory at `vaddr..vaddr+len` within the process user space using `op`.
    ///
    /// The corresponding `VmSpace` is not required be activated on the current CPU.
    /// If any page in the range is not mapped or does not have the required page
    /// flags, a page fault will be handled to try to make the page accessible.
    fn access_remote<F>(
        &self,
        vaddr: Vaddr,
        len: usize,
        required_page_flags: PageFlags,
        mut op: F,
    ) -> Result<usize>
    where
        F: FnMut(UFrame, usize) -> core::result::Result<usize, (ostd::Error, usize)>,
    {
        if len == 0 {
            return Ok(0);
        }

        let range = {
            let Some(end) = vaddr.checked_add(len) else {
                return_errno_with_message!(Errno::EINVAL, "address overflow");
            };
            if !is_userspace_vaddr(vaddr) || !is_userspace_vaddr(end - 1) {
                return_errno_with_message!(Errno::EINVAL, "invalid user space address");
            }
            vaddr.align_down(PAGE_SIZE)..end.align_up(PAGE_SIZE)
        };

        let mut current_va = range.start;
        let mut bytes = 0;

        while current_va < range.end {
            let mut item = self.query_page(current_va, None)?;

            if item
                .as_ref()
                .is_none_or(|item| !item.prop().flags.contains(required_page_flags))
            {
                let vmar_guard = self.lock_root_vmar();
                let vmar = vmar_guard
                    .as_ref()
                    .ok_or_else(|| Error::with_message(Errno::ESRCH, "the process has exited"))?;

                let page_fault_info = PageFaultInfo {
                    address: current_va,
                    required_perms: required_page_flags.into(),
                };
                if vmar.handle_page_fault(&page_fault_info).is_err() {
                    if bytes == 0 {
                        return_errno_with_message!(Errno::EIO, "the page is not accessible");
                    } else {
                        return Ok(bytes);
                    }
                };

                item = self.query_page(current_va, Some(vmar_guard))?;
            }

            let item = item.unwrap();
            debug_assert!(item.prop().flags.contains(required_page_flags));

            match item {
                VmQueriedItem::MappedRam { frame, .. } => {
                    let skip_offset = if current_va == range.start {
                        vaddr - range.start
                    } else {
                        0
                    };
                    match op(frame, skip_offset) {
                        Ok(n) => bytes += n,
                        Err((err, n)) if bytes + n == 0 => return Err(Error::from(err)),
                        Err((_, n)) => return Ok(bytes + n),
                    }
                }
                VmQueriedItem::MappedIoMem { .. } => {
                    return_errno_with_message!(
                        Errno::EOPNOTSUPP,
                        "accessing remote MMIO memory is not supported currently"
                    );
                }
            }

            current_va += PAGE_SIZE;
        }

        Ok(bytes)
    }

    fn query_page(
        &self,
        vaddr: Vaddr,
        vmar_guard: Option<ProcessVmarGuard>,
    ) -> Result<Option<VmQueriedItem>> {
        debug_assert!(is_userspace_vaddr(vaddr) && vaddr % PAGE_SIZE == 0);

        let vmar_guard = vmar_guard.unwrap_or_else(|| self.lock_root_vmar());
        let vmar = vmar_guard
            .as_ref()
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "the process has exited"))?;

        let preempt_guard = disable_preempt();
        let vmspace = vmar.vm_space();
        let mut cursor = vmspace.cursor(&preempt_guard, &(vaddr..vaddr + PAGE_SIZE))?;
        let (_, item) = cursor.query()?;

        Ok(item)
    }
}

/// Renews the [`ProcessVm`] of the current process and then maps the heap VMO to the new VMAR.
pub fn renew_vm_and_map(ctx: &Context) {
    let process_vm = ctx.process.vm();
    let mut root_vmar = process_vm.lock_root_vmar();

    let new_vmar = Vmar::<Full>::new_root();
    let guard = disable_preempt();
    *ctx.thread_local.root_vmar().borrow_mut() = Some(new_vmar.dup().unwrap());
    new_vmar.vm_space().activate();
    root_vmar.set_vmar(Some(new_vmar));
    drop(guard);

    process_vm
        .heap
        .alloc_and_map_vm(root_vmar.unwrap())
        .unwrap();
}
