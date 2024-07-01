// SPDX-License-Identifier: MPL-2.0

//! I/O memory.

use core::{mem::size_of, ops::Range};

use crate::{
    mm::{kspace::LINEAR_MAPPING_BASE_VADDR, paddr_to_vaddr, HasPaddr, Paddr, Vaddr, VmIo},
    Error, Pod, Result,
};

/// I/O memory.
#[derive(Debug, Clone)]
pub struct IoMem {
    virtual_address: Vaddr,
    limit: usize,
}

impl VmIo for IoMem {
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> crate::Result<()> {
        self.check_range(offset, buf.len())?;
        unsafe {
            core::ptr::copy(
                (self.virtual_address + offset) as *const u8,
                buf.as_mut_ptr(),
                buf.len(),
            );
        }
        Ok(())
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> crate::Result<()> {
        self.check_range(offset, buf.len())?;
        unsafe {
            core::ptr::copy(
                buf.as_ptr(),
                (self.virtual_address + offset) as *mut u8,
                buf.len(),
            );
        }
        Ok(())
    }

    fn read_val<T: Pod>(&self, offset: usize) -> crate::Result<T> {
        self.check_range(offset, size_of::<T>())?;
        Ok(unsafe { core::ptr::read_volatile((self.virtual_address + offset) as *const T) })
    }

    fn write_val<T: Pod>(&self, offset: usize, new_val: &T) -> crate::Result<()> {
        self.check_range(offset, size_of::<T>())?;
        unsafe { core::ptr::write_volatile((self.virtual_address + offset) as *mut T, *new_val) };
        Ok(())
    }
}

impl HasPaddr for IoMem {
    fn paddr(&self) -> Paddr {
        self.virtual_address - LINEAR_MAPPING_BASE_VADDR
    }
}

impl IoMem {
    /// Creates a new `IoMem`.
    ///
    /// # Safety
    ///
    /// User must ensure the given physical range is in the I/O memory region.
    pub(crate) unsafe fn new(range: Range<Paddr>) -> IoMem {
        IoMem {
            virtual_address: paddr_to_vaddr(range.start),
            limit: range.len(),
        }
    }

    /// Returns the physical address of the I/O memory.
    pub fn paddr(&self) -> Paddr {
        self.virtual_address - LINEAR_MAPPING_BASE_VADDR
    }

    /// Returns the length of the I/O memory region.
    pub fn length(&self) -> usize {
        self.limit
    }

    /// Resizes the I/O memory region to the new `range`.
    ///
    /// # Errors
    ///
    /// Returns an error if the new `range` is not within the current range.
    pub fn resize(&mut self, range: Range<Paddr>) -> Result<()> {
        let start_vaddr = paddr_to_vaddr(range.start);
        let virtual_end = self
            .virtual_address
            .checked_add(self.limit)
            .ok_or(Error::Overflow)?;
        if start_vaddr < self.virtual_address || start_vaddr >= virtual_end {
            return Err(Error::InvalidArgs);
        }
        let end_vaddr = start_vaddr
            .checked_add(range.len())
            .ok_or(Error::Overflow)?;
        if end_vaddr <= self.virtual_address || end_vaddr > virtual_end {
            return Err(Error::InvalidArgs);
        }
        self.virtual_address = start_vaddr;
        self.limit = range.len();
        Ok(())
    }

    fn check_range(&self, offset: usize, len: usize) -> Result<()> {
        let sum = offset.checked_add(len).ok_or(Error::InvalidArgs)?;
        if sum > self.limit {
            log::error!(
                "attempt to access address out of bounds, limit:0x{:x}, access position:0x{:x}",
                self.limit,
                sum
            );
            Err(Error::InvalidArgs)
        } else {
            Ok(())
        }
    }
}
