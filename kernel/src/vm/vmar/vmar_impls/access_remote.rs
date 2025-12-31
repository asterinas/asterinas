// SPDX-License-Identifier: MPL-2.0

use align_ext::AlignExt;
use ostd::{
    mm::{PAGE_SIZE, PageFlags, UFrame, io_util::HasVmReaderWriter, vm_space::VmQueriedItem},
    task::disable_preempt,
};

use super::Vmar;
use crate::{
    prelude::*,
    thread::exception::PageFaultInfo,
    vm::vmar::{is_userspace_vaddr, is_userspace_vaddr_range},
};

impl Vmar {
    /// Reads memory from the process user space.
    ///
    /// This method reads until one of the conditions is met:
    /// 1. The writer has no available space.
    /// 2. Reading from the process user space or writing to the writer encounters some error.
    ///
    /// On success, the number of bytes read is returned;
    /// On error, both the error and the number of bytes read so far are returned.
    ///
    /// The `VmSpace` of the process is not required to be activated on the current CPU.
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
    /// The `VmSpace` of the process is not required to be activated on the current CPU.
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

    /// Writes zeros to the process user space.
    ///
    /// This method writes at most `len` bytes of zeros to the process user space.
    /// On success, the number of bytes written is returned; on error, both the
    /// error and the number of bytes written so far are returned.
    ///
    /// The `VmSpace` of the process is not required to be activated on the current CPU.
    pub fn fill_zeros_remote(
        &self,
        vaddr: Vaddr,
        len: usize,
    ) -> core::result::Result<usize, (Error, usize)> {
        let mut remain = len;
        let write = |frame: UFrame, skip_offset: usize| {
            let mut writer = frame.writer();
            writer.skip(skip_offset);
            let res = writer.fill_zeros(remain);
            remain -= res;
            Ok(res)
        };

        self.access_remote(vaddr, len, PageFlags::W, write)
    }

    /// Accesses memory at `vaddr..vaddr+len` within the process user space using `op`.
    ///
    /// The `VmSpace` of the process is not required to be activated on the current CPU.
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

        if !is_userspace_vaddr_range(vaddr, len) {
            return Err((
                Error::with_message(Errno::EINVAL, "the address range is not in userspace"),
                0,
            ));
        }
        let range = vaddr.align_down(PAGE_SIZE)..(vaddr + len).align_up(PAGE_SIZE);

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
        debug_assert!(is_userspace_vaddr(vaddr) && vaddr.is_multiple_of(PAGE_SIZE));

        let vmspace = self.vm_space();

        loop {
            let preempt_guard = disable_preempt();
            let mut cursor = vmspace.cursor(&preempt_guard, &(vaddr..vaddr + PAGE_SIZE))?;

            match cursor.query() {
                VmQueriedItem::MappedRam { frame, prop }
                    if prop.flags.contains(required_page_flags) =>
                {
                    return Ok((*frame).clone());
                }
                VmQueriedItem::MappedIoMem { .. } => {
                    return_errno_with_message!(
                        Errno::EOPNOTSUPP,
                        "accessing remote MMIO memory is not supported currently"
                    );
                }
                _ => {}
            }

            drop(cursor);
            drop(preempt_guard);

            let page_fault_info = PageFaultInfo {
                address: vaddr,
                required_perms: required_page_flags.into(),
            };
            self.handle_page_fault(&page_fault_info)?;

            // Note that we are not holding `self.inner.lock()` here. Therefore, in race conditions
            // (e.g., if the mapping is removed concurrently), we will need to try again. The same
            // is true for real page faults; they may occur more than once at the same address.
        }
    }
}
