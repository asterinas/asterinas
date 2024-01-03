// SPDX-License-Identifier: MPL-2.0

use crate::arch::mm::PageTableFlags;
use crate::config::PAGE_SIZE;
use crate::sync::Mutex;
use bitflags::bitflags;
use core::ops::Range;

use super::VmFrameVec;
use super::{is_page_aligned, Vaddr};
use super::{MapArea, MemorySet};
use crate::{prelude::*, Error};

use super::VmIo;

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
/// physical memory (`VmFrames`) to the `VmSpace`.

#[derive(Debug, Clone)]
pub struct VmSpace {
    memory_set: Arc<Mutex<MemorySet>>,
}

impl VmSpace {
    /// Creates a new VM address space.
    pub fn new() -> Self {
        Self {
            memory_set: Arc::new(Mutex::new(MemorySet::new())),
        }
    }
    /// Activate the page table, load root physical address to cr3
    #[allow(clippy::missing_safety_doc)]
    pub unsafe fn activate(&self) {
        #[cfg(target_arch = "x86_64")]
        crate::arch::x86::mm::activate_page_table(
            self.memory_set.lock().pt.root_paddr(),
            x86_64::registers::control::Cr3Flags::PAGE_LEVEL_CACHE_DISABLE,
        );
    }

    /// Maps some physical memory pages into the VM space according to the given
    /// options, returning the address where the mapping is created.
    ///
    /// the frames in variable frames will delete after executing this function
    ///
    /// For more information, see `VmMapOptions`.
    pub fn map(&self, frames: VmFrameVec, options: &VmMapOptions) -> Result<Vaddr> {
        let flags = PageTableFlags::from(options.perm);
        if options.addr.is_none() {
            return Err(Error::InvalidArgs);
        }

        // if can overwrite, the old mapping should be unmapped.
        if options.can_overwrite {
            let addr = options.addr.unwrap();
            let size = frames.nbytes();
            let _ = self.unmap(&(addr..addr + size));
        }
        // debug!("map to vm space: 0x{:x}", options.addr.unwrap());

        let mut memory_set = self.memory_set.lock();
        // FIXME: This is only a hack here. The interface of MapArea cannot simply deal with unmap part of memory,
        // so we only map MapArea of page size now.
        for (idx, frame) in frames.into_iter().enumerate() {
            let addr = options.addr.unwrap() + idx * PAGE_SIZE;
            let frames = VmFrameVec::from_one_frame(frame);
            memory_set.map(MapArea::new(addr, PAGE_SIZE, flags, frames));
        }

        Ok(options.addr.unwrap())
    }

    /// determine whether a vaddr is already mapped
    pub fn is_mapped(&self, vaddr: Vaddr) -> bool {
        let memory_set = self.memory_set.lock();
        memory_set.is_mapped(vaddr)
    }

    /// Unmaps the physical memory pages within the VM address range.
    ///
    /// The range is allowed to contain gaps, where no physical memory pages
    /// are mapped.
    pub fn unmap(&self, range: &Range<Vaddr>) -> Result<()> {
        assert!(is_page_aligned(range.start) && is_page_aligned(range.end));
        let mut start_va = range.start;
        let page_size = (range.end - range.start) / PAGE_SIZE;
        let mut inner = self.memory_set.lock();
        for i in 0..page_size {
            inner.unmap(start_va)?;
            start_va += PAGE_SIZE;
        }
        Ok(())
    }

    /// clear all mappings
    pub fn clear(&self) {
        self.memory_set.lock().clear();
        #[cfg(target_arch = "x86_64")]
        x86_64::instructions::tlb::flush_all();
    }

    /// Update the VM protection permissions within the VM address range.
    ///
    /// The entire specified VM range must have been mapped with physical
    /// memory pages.
    pub fn protect(&self, range: &Range<Vaddr>, perm: VmPerm) -> Result<()> {
        debug_assert!(range.start % PAGE_SIZE == 0);
        debug_assert!(range.end % PAGE_SIZE == 0);
        let start_page = range.start / PAGE_SIZE;
        let end_page = range.end / PAGE_SIZE;
        let flags = PageTableFlags::from(perm);
        for page_idx in start_page..end_page {
            let addr = page_idx * PAGE_SIZE;
            self.memory_set.lock().protect(addr, flags)
        }
        Ok(())
    }
}

impl Default for VmSpace {
    fn default() -> Self {
        Self::new()
    }
}

impl VmIo for VmSpace {
    fn read_bytes(&self, vaddr: usize, buf: &mut [u8]) -> Result<()> {
        self.memory_set.lock().read_bytes(vaddr, buf)
    }

    fn write_bytes(&self, vaddr: usize, buf: &[u8]) -> Result<()> {
        self.memory_set.lock().write_bytes(vaddr, buf)
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
    /// permission
    perm: VmPerm,
    /// can overwrite
    can_overwrite: bool,
}

impl VmMapOptions {
    /// Creates the default options.
    pub fn new() -> Self {
        Self {
            addr: None,
            align: PAGE_SIZE,
            perm: VmPerm::empty(),
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
    pub fn perm(&mut self, perm: VmPerm) -> &mut Self {
        self.perm = perm;
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

bitflags! {
    /// Virtual memory protection permissions.
    pub struct VmPerm: u8 {
        /// Readable.
        const R = 0b00000001;
        /// Writable.
        const W = 0b00000010;
        /// Executable.
        const X = 0b00000100;
        /// User
        const U = 0b00001000;
        /// Readable + writable.
        const RW = Self::R.bits | Self::W.bits;
        /// Readable + execuable.
        const RX = Self::R.bits | Self::X.bits;
        /// Readable + writable + executable.
        const RWX = Self::R.bits | Self::W.bits | Self::X.bits;
        /// Readable + writable + user.
        const RWU = Self::R.bits | Self::W.bits | Self::U.bits;
        /// Readable + execuable + user.
        const RXU = Self::R.bits | Self::X.bits | Self::U.bits;
        /// Readable + writable + executable + user.
        const RWXU = Self::R.bits | Self::W.bits | Self::X.bits | Self::U.bits;
    }
}

impl TryFrom<u64> for VmPerm {
    type Error = Error;

    fn try_from(value: u64) -> Result<Self> {
        VmPerm::from_bits(value as u8).ok_or(Error::InvalidArgs)
    }
}

impl From<VmPerm> for PageTableFlags {
    fn from(vm_perm: VmPerm) -> Self {
        let mut flags = PageTableFlags::PRESENT | PageTableFlags::USER;
        if vm_perm.contains(VmPerm::W) {
            flags |= PageTableFlags::WRITABLE;
        }
        // FIXME: how to respect executable flags?
        if !vm_perm.contains(VmPerm::X) {
            flags |= PageTableFlags::NO_EXECUTE;
        }
        flags
    }
}
