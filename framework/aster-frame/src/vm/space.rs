// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use align_ext::AlignExt;

use super::{
    is_page_aligned,
    kspace::KERNEL_PAGE_TABLE,
    page_table::{PageTable, PageTableMode, UserMode},
    CachePolicy, PageFlags, PageProperty, PagingConstsTrait, PrivilegedPageFlags, VmFrameVec, VmIo,
    PAGE_SIZE,
};
use crate::{
    arch::mm::{PageTableEntry, PagingConsts},
    prelude::*,
    vm::{
        page_table::{Cursor, PageTableQueryResult as PtQr},
        VmFrame, MAX_USERSPACE_VADDR,
    },
    Error,
};

/// Virtual memory space.
///
/// A virtual memory space (`VmSpace`) can be created and assigned to a user space so that
/// the virtual memory of the user space can be manipulated safely. For example,
/// given an arbitrary user-space pointer, one can read and write the memory
/// location refered to by the user-space pointer without the risk of breaking the
/// memory safety of the kernel space.
///
/// A newly-created `VmSpace` is not backed by any physical memory pages.
/// To provide memory pages for a `VmSpace`, one can allocate and map
/// physical memory (`VmFrame`s) to the `VmSpace`.
#[derive(Debug)]
pub struct VmSpace {
    pt: PageTable<UserMode>,
}

impl VmSpace {
    /// Creates a new VM address space.
    pub fn new() -> Self {
        Self {
            pt: KERNEL_PAGE_TABLE.get().unwrap().create_user_page_table(),
        }
    }
    /// Activate the page table.
    pub(crate) fn activate(&self) {
        self.pt.activate();
    }

    /// Maps some physical memory pages into the VM space according to the given
    /// options, returning the address where the mapping is created.
    ///
    /// The ownership of the frames will be transferred to the `VmSpace`.
    ///
    /// For more information, see `VmMapOptions`.
    pub fn map(&self, frames: VmFrameVec, options: &VmMapOptions) -> Result<Vaddr> {
        if options.addr.is_none() {
            return Err(Error::InvalidArgs);
        }

        let addr = options.addr.unwrap();

        if addr % PAGE_SIZE != 0 {
            return Err(Error::InvalidArgs);
        }

        let size = frames.nbytes();
        let end = addr.checked_add(size).ok_or(Error::InvalidArgs)?;

        let va_range = addr..end;
        if !UserMode::covers(&va_range) {
            return Err(Error::InvalidArgs);
        }

        let mut cursor = self.pt.cursor_mut(&va_range)?;

        // If overwrite is forbidden, we should check if there are existing mappings
        if !options.can_overwrite {
            while let Some(qr) = cursor.query() {
                if matches!(qr, PtQr::Mapped { .. }) {
                    return Err(Error::MapAlreadyMappedVaddr);
                }
            }
            cursor.jump(va_range.start);
        }

        let prop = PageProperty {
            flags: options.flags,
            cache: CachePolicy::Writeback,
            priv_flags: PrivilegedPageFlags::USER,
        };

        for frame in frames.into_iter() {
            // SAFETY: mapping in the user space with `VmFrame` is safe.
            unsafe {
                cursor.map(frame, prop);
            }
        }

        Ok(addr)
    }

    /// Query about a range of virtual memory.
    /// You will get a iterator of `VmQueryResult` which contains the information of
    /// each parts of the range.
    pub fn query_range(&self, range: &Range<Vaddr>) -> Result<VmQueryIter> {
        Ok(VmQueryIter {
            cursor: self.pt.cursor(range)?,
        })
    }

    /// Query about the mapping information about a byte in virtual memory.
    /// This is more handy than [`query_range`], but less efficient if you want
    /// to query in a batch.
    pub fn query(&self, vaddr: Vaddr) -> Result<Option<PageProperty>> {
        if !(0..MAX_USERSPACE_VADDR).contains(&vaddr) {
            return Err(Error::AccessDenied);
        }
        Ok(self.pt.query(vaddr).map(|(_pa, prop)| prop))
    }

    /// Unmaps the physical memory pages within the VM address range.
    ///
    /// The range is allowed to contain gaps, where no physical memory pages
    /// are mapped.
    pub fn unmap(&self, range: &Range<Vaddr>) -> Result<()> {
        if !is_page_aligned(range.start) || !is_page_aligned(range.end) {
            return Err(Error::InvalidArgs);
        }
        if !UserMode::covers(range) {
            return Err(Error::InvalidArgs);
        }
        // SAFETY: unmapping in the user space is safe.
        unsafe {
            self.pt.unmap(range)?;
        }
        Ok(())
    }

    /// clear all mappings
    pub fn clear(&self) {
        // SAFETY: unmapping user space is safe, and we don't care unmapping
        // invalid ranges.
        unsafe {
            self.pt.unmap(&(0..MAX_USERSPACE_VADDR)).unwrap();
        }
        #[cfg(target_arch = "x86_64")]
        x86_64::instructions::tlb::flush_all();
    }

    /// Update the VM protection permissions within the VM address range.
    ///
    /// If any of the page in the given range is not mapped, it is skipped.
    /// The method panics when virtual address is not aligned to base page
    /// size.
    ///
    /// It is guarenteed that the operation is called once for each valid
    /// page found in the range.
    ///
    /// TODO: It returns error when invalid operations such as protect
    /// partial huge page happens, and efforts are not reverted, leaving us
    /// in a bad state.
    pub fn protect(&self, range: &Range<Vaddr>, op: impl FnMut(&mut PageProperty)) -> Result<()> {
        if !is_page_aligned(range.start) || !is_page_aligned(range.end) {
            return Err(Error::InvalidArgs);
        }
        if !UserMode::covers(range) {
            return Err(Error::InvalidArgs);
        }
        // SAFETY: protecting in the user space is safe.
        unsafe {
            self.pt.protect(range, op)?;
        }
        Ok(())
    }

    /// To fork a new VM space with copy-on-write semantics.
    ///
    /// Both the parent and the newly forked VM space will be marked as
    /// read-only. And both the VM space will take handles to the same
    /// physical memory pages.
    pub fn fork_copy_on_write(&self) -> Self {
        Self {
            pt: self.pt.fork_copy_on_write(),
        }
    }
}

impl Default for VmSpace {
    fn default() -> Self {
        Self::new()
    }
}

impl VmIo for VmSpace {
    fn read_bytes(&self, vaddr: usize, buf: &mut [u8]) -> Result<()> {
        let range_end = vaddr.checked_add(buf.len()).ok_or(Error::Overflow)?;
        let vaddr = vaddr.align_down(PAGE_SIZE);
        let range_end = range_end.align_up(PAGE_SIZE);
        for qr in self.query_range(&(vaddr..range_end))? {
            if matches!(qr, VmQueryResult::NotMapped { .. }) {
                return Err(Error::AccessDenied);
            }
        }
        self.activate();
        buf.clone_from_slice(unsafe { core::slice::from_raw_parts(vaddr as *const u8, buf.len()) });
        Ok(())
    }

    fn write_bytes(&self, vaddr: usize, buf: &[u8]) -> Result<()> {
        let range_end = vaddr.checked_add(buf.len()).ok_or(Error::Overflow)?;
        let vaddr = vaddr.align_down(PAGE_SIZE);
        let range_end = range_end.align_up(PAGE_SIZE);
        for qr in self.query_range(&(vaddr..vaddr + range_end))? {
            match qr {
                VmQueryResult::NotMapped { .. } => return Err(Error::AccessDenied),
                VmQueryResult::Mapped { prop, .. } => {
                    if !prop.flags.contains(PageFlags::W) {
                        return Err(Error::AccessDenied);
                    }
                }
            }
        }
        self.activate();
        unsafe { core::slice::from_raw_parts_mut(vaddr as *mut u8, buf.len()) }
            .clone_from_slice(buf);
        Ok(())
    }
}

/// Options for mapping physical memory pages into a VM address space.
/// See `VmSpace::map`.
#[derive(Clone, Debug)]
pub struct VmMapOptions {
    /// start virtual address
    addr: Option<Vaddr>,
    /// map align
    align: usize,
    /// page permissions and status
    flags: PageFlags,
    /// can overwrite
    can_overwrite: bool,
}

impl VmMapOptions {
    /// Creates the default options.
    pub fn new() -> Self {
        Self {
            addr: None,
            align: PagingConsts::BASE_PAGE_SIZE,
            flags: PageFlags::empty(),
            can_overwrite: false,
        }
    }

    /// Sets the alignment of the address of the mapping.
    ///
    /// The alignment must be a power-of-2 and greater than or equal to the
    /// page size.
    ///
    /// The default value of this option is the page size.
    pub fn align(&mut self, align: usize) -> &mut Self {
        self.align = align;
        self
    }

    /// Sets the permissions of the mapping, which affects whether
    /// the mapping can be read, written, or executed.
    ///
    /// The default value of this option is read-only.
    pub fn flags(&mut self, flags: PageFlags) -> &mut Self {
        self.flags = flags;
        self
    }

    /// Sets the address of the new mapping.
    ///
    /// The default value of this option is `None`.
    pub fn addr(&mut self, addr: Option<Vaddr>) -> &mut Self {
        if addr.is_none() {
            return self;
        }
        self.addr = Some(addr.unwrap());
        self
    }

    /// Sets whether the mapping can overwrite any existing mappings.
    ///
    /// If this option is `true`, then the address option must be `Some(_)`.
    ///
    /// The default value of this option is `false`.
    pub fn can_overwrite(&mut self, can_overwrite: bool) -> &mut Self {
        self.can_overwrite = can_overwrite;
        self
    }
}

impl Default for VmMapOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// The iterator for querying over the VM space without modifying it.
pub struct VmQueryIter<'a> {
    cursor: Cursor<'a, UserMode, PageTableEntry, PagingConsts>,
}

pub enum VmQueryResult {
    NotMapped {
        va: Vaddr,
        len: usize,
    },
    Mapped {
        va: Vaddr,
        frame: VmFrame,
        prop: PageProperty,
    },
}

impl Iterator for VmQueryIter<'_> {
    type Item = VmQueryResult;

    fn next(&mut self) -> Option<Self::Item> {
        self.cursor.next().map(|ptqr| match ptqr {
            PtQr::NotMapped { va, len } => VmQueryResult::NotMapped { va, len },
            PtQr::Mapped { va, frame, prop } => VmQueryResult::Mapped { va, frame, prop },
            // It is not possible to map untyped memory in user space.
            PtQr::MappedUntyped { .. } => unreachable!(),
        })
    }
}
