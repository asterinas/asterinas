use bitflags::bitflags;
use core::ops::Range;

use crate::prelude::*;
use crate::vm::VmFrameVec;

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
pub struct VmSpace {}

impl VmSpace {
    /// Creates a new VM address space.
    pub fn new() -> Self {
        todo!()
    }

    /// Maps some physical memory pages into the VM space according to the given
    /// options, returning the address where the mapping is created.
    ///
    /// For more information, see `VmMapOptions`.
    pub fn map(&self, frames: VmFrameVec, options: &VmMapOptions) -> Result<Vaddr> {
        todo!()
    }

    /// Unmaps the physical memory pages within the VM address range.
    ///
    /// The range is allowed to contain gaps, where no physical memory pages
    /// are mapped.
    pub fn unmap(&self, range: &Range<Vaddr>) -> Result<()> {
        todo!()
    }

    /// Update the VM protection permissions within the VM address range.
    ///
    /// The entire specified VM range must have been mapped with physical
    /// memory pages.
    pub fn protect(&self, range: &Range<Vaddr>, perm: VmPerm) -> Result<()> {
        todo!()
    }
}

impl Default for VmSpace {
    fn default() -> Self {
        Self::new()
    }
}

impl VmIo for VmSpace {
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        todo!()
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        todo!()
    }
}

/// Options for mapping physical memory pages into a VM address space.
/// See `VmSpace::map`.
pub struct VmMapOptions {}

impl VmMapOptions {
    /// Creates the default options.
    pub fn new() -> Self {
        todo!()
    }

    /// Sets the alignment of the address of the mapping.
    ///
    /// The alignment must be a power-of-2 and greater than or equal to the
    /// page size.
    ///
    /// The default value of this option is the page size.
    pub fn align(&mut self, align: usize) -> &mut Self {
        todo!()
    }

    /// Sets the permissions of the mapping, which affects whether
    /// the mapping can be read, written, or executed.
    ///
    /// The default value of this option is read-only.
    pub fn perm(&mut self, perm: VmPerm) -> &mut Self {
        todo!()
    }

    /// Sets the address of the new mapping.
    ///
    /// The default value of this option is `None`.
    pub fn addr(&mut self, addr: Option<Vaddr>) -> &mut Self {
        todo!()
    }

    /// Sets whether the mapping can overwrite any existing mappings.
    ///
    /// If this option is `true`, then the address option must be `Some(_)`.
    ///
    /// The default value of this option is `false`.
    pub fn can_overwrite(&mut self, can_overwrite: bool) -> &mut Self {
        todo!()
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
        /// Readable + writable.
        const RW = Self::R.bits | Self::W.bits;
        /// Readable + execuable.
        const RX = Self::R.bits | Self::X.bits;
        /// Readable + writable + executable.
        const RWX = Self::R.bits | Self::W.bits | Self::X.bits;
    }
}
