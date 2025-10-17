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
pub struct ProcessVm(Mutex<Option<Vmar<Full>>>);

/// A guard to the [`Vmar`] used by a process.
///
/// It is bound to a [`ProcessVm`] and can only be obtained from
/// the [`ProcessVm::lock_vmar`] method.
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
        let vmar = self.lock_vmar();
        Self(Mutex::new(Some(vmar.unwrap().dup().unwrap())))
    }
}

impl ProcessVm {
    /// Allocates a new `ProcessVm`
    pub fn alloc() -> Self {
        let vmar = Vmar::<Full>::new();
        let heap = vmar.heap();
        heap.alloc_and_map(&vmar).unwrap();
        Self(Mutex::new(Some(vmar)))
    }

    /// Forks a `ProcessVm` from `other`.
    ///
    /// The returned `ProcessVm` will have a forked `Vmar`.
    pub fn fork_from(other: &ProcessVm) -> Result<Self> {
        let process_vmar = other.lock_vmar();
        let vmar = Mutex::new(Some(Vmar::<Full>::fork_from(process_vmar.unwrap())?));
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
impl Vmar<Full> {
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

// TODO: Move the below code to the vm module.
impl Vmar<Full> {
    /// Reads memory from the process user space.
    ///
    /// This method reads until one of the conditions is met:
    /// 1. The writer has no available space.
    /// 2. Reading from the process user space or writing to the writer encounters some error.
    ///
    /// On success, the number of bytes read is returned;
    /// On error, both the error and the number of bytes read so far are returned.
    ///
    /// The `VmSpace` of the process is not required be activated on the current CPU.
    pub fn read_remote(
        &self,
        vaddr: Vaddr,
        writer: &mut VmWriter,
    ) -> core::result::Result<usize, (Error, usize)> {
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
    /// This method writes until one of the conditions is met:
    /// 1. The reader has no remaining data.
    /// 2. Reading from the reader or writing to the process user space encounters some error.
    ///
    /// On success, the number of bytes written is returned;
    /// On error, both the error and the number of bytes written so far are returned.
    ///
    /// The `VmSpace` of the process is not required be activated on the current CPU.
    pub fn write_remote(
        &self,
        vaddr: Vaddr,
        reader: &mut VmReader,
    ) -> core::result::Result<usize, (Error, usize)> {
        let len = reader.remain();
        let write = |frame: UFrame, skip_offset: usize| {
            let mut writer = frame.writer();
            writer.skip(skip_offset);
            writer.write_fallible(reader)
        };

        self.access_remote(vaddr, len, PageFlags::W, write)
    }

    /// Accesses memory at `vaddr..vaddr+len` within the process user space using `op`.
    ///
    /// The `VmSpace` of the process is not required be activated on the current CPU.
    /// If any page in the range is not mapped or does not have the required page
    /// flags, a page fault will be handled to try to make the page accessible.
    fn access_remote<F>(
        &self,
        vaddr: Vaddr,
        len: usize,
        required_page_flags: PageFlags,
        mut op: F,
    ) -> core::result::Result<usize, (Error, usize)>
    where
        F: FnMut(UFrame, usize) -> core::result::Result<usize, (ostd::Error, usize)>,
    {
        if len == 0 {
            return Ok(0);
        }

        let range = check_userspace_page_range(vaddr, len).map_err(|err| (err, 0))?;

        let mut current_va = range.start;
        let mut bytes = 0;

        while current_va < range.end {
            let frame = self
                .query_page_with_required_flags(current_va, required_page_flags)
                .map_err(|err| (err, bytes))?;

            let skip_offset = if current_va == range.start {
                vaddr - range.start
            } else {
                0
            };
            match op(frame, skip_offset) {
                Ok(n) => bytes += n,
                Err((err, n)) => return Err((err.into(), bytes + n)),
            }

            current_va += PAGE_SIZE;
        }

        Ok(bytes)
    }

    fn query_page_with_required_flags(
        &self,
        vaddr: Vaddr,
        required_page_flags: PageFlags,
    ) -> Result<UFrame> {
        let mut item = self.query_page(vaddr)?;

        if item
            .as_ref()
            .is_none_or(|item| !item.prop().flags.contains(required_page_flags))
        {
            let page_fault_info = PageFaultInfo {
                address: vaddr,
                required_perms: required_page_flags.into(),
            };
            self.handle_page_fault(&page_fault_info)
                .map_err(|_| Error::with_message(Errno::EIO, "the page is not accessible"))?;

            item = self.query_page(vaddr)?;
        }

        let item = item.unwrap();
        debug_assert!(item.prop().flags.contains(required_page_flags));

        match item {
            VmQueriedItem::MappedRam { frame, .. } => Ok(frame),
            VmQueriedItem::MappedIoMem { .. } => {
                return_errno_with_message!(
                    Errno::EOPNOTSUPP,
                    "accessing remote MMIO memory is not supported currently"
                );
            }
        }
    }

    fn query_page(&self, vaddr: Vaddr) -> Result<Option<VmQueriedItem>> {
        debug_assert!(is_userspace_vaddr(vaddr) && vaddr % PAGE_SIZE == 0);

        let preempt_guard = disable_preempt();
        let vmspace = self.vm_space();
        let mut cursor = vmspace.cursor(&preempt_guard, &(vaddr..vaddr + PAGE_SIZE))?;
        let (_, item) = cursor.query()?;

        Ok(item)
    }
}

/// Unshares and renews the [`Vmar`] of the current process.
pub(super) fn unshare_and_renew_vmar(ctx: &Context, vmar: &mut ProcessVmarGuard) {
    let new_vmar = Vmar::<Full>::new();
    let guard = disable_preempt();
    *ctx.thread_local.vmar().borrow_mut() = Some(new_vmar.dup().unwrap());
    new_vmar.vm_space().activate();
    vmar.set_vmar(Some(new_vmar));
    drop(guard);

    let new_vmar = vmar.unwrap();
    new_vmar.heap().alloc_and_map(new_vmar).unwrap();
}

fn check_userspace_page_range(vaddr: Vaddr, len: usize) -> Result<Range<Vaddr>> {
    let Some(end) = vaddr.checked_add(len) else {
        return_errno_with_message!(Errno::EINVAL, "address overflow");
    };
    if !is_userspace_vaddr(vaddr) || !is_userspace_vaddr(end - 1) {
        return_errno_with_message!(Errno::EINVAL, "invalid user space address");
    }
    Ok(vaddr.align_down(PAGE_SIZE)..end.align_up(PAGE_SIZE))
}
