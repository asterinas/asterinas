// SPDX-License-Identifier: MPL-2.0

//! I/O memory.

use core::ops::Range;

use crate::{
    mm::{
        kspace::LINEAR_MAPPING_BASE_VADDR, paddr_to_vaddr, FallibleVmRead, FallibleVmWrite,
        HasPaddr, Infallible, Paddr, PodOnce, Vaddr, VmIo, VmIoOnce, VmReader, VmWriter,
    },
    prelude::*,
    Error,
};

/// I/O memory.
#[derive(Debug, Clone)]
pub struct IoMem {
    virtual_address: Vaddr,
    limit: usize,
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
    /// - The given physical address range must be in the I/O memory region.
    /// - Reading from or writing to I/O memory regions may have side effects. Those side effects
    ///   must not cause soundness problems (e.g., they must not corrupt the kernel memory).
    pub(crate) unsafe fn new(range: Range<Paddr>) -> Self {
        Self {
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

    /// Slices the `IoMem`, returning another `IoMem` representing the subslice.
    ///
    /// # Panics
    ///
    /// This method will panic if the range is empty or out of bounds.
    pub fn slice(&self, range: Range<usize>) -> Self {
        // This ensures `range.start < range.end` and `range.end <= limit`.
        assert!(!range.is_empty() && range.end <= self.limit);

        // We've checked the range is in bounds, so we can construct the new `IoMem` safely.
        Self {
            virtual_address: self.virtual_address + range.start,
            limit: range.end - range.start,
        }
    }
}

// For now, we reuse `VmReader` and `VmWriter` to access I/O memory.
//
// Note that I/O memory is not normal typed or untyped memory. Strictly speaking, it is not
// "memory", but rather I/O ports that communicate directly with the hardware. However, this code
// is in OSTD, so we can rely on the implementation details of `VmReader` and `VmWriter`, which we
// know are also suitable for accessing I/O memory.
impl IoMem {
    fn reader(&self) -> VmReader<'_, Infallible> {
        // SAFETY: The safety conditions of `IoMem::new` guarantee we can read from the I/O memory
        // safely.
        unsafe { VmReader::from_kernel_space(self.virtual_address as *mut u8, self.limit) }
    }

    fn writer(&self) -> VmWriter<'_, Infallible> {
        // SAFETY: The safety conditions of `IoMem::new` guarantee we can read from the I/O memory
        // safely.
        unsafe { VmWriter::from_kernel_space(self.virtual_address as *mut u8, self.limit) }
    }
}

impl VmIo for IoMem {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()> {
        if self
            .limit
            .checked_sub(offset)
            .is_none_or(|remain| remain < writer.avail())
        {
            return Err(Error::InvalidArgs);
        }

        self.reader()
            .skip(offset)
            .read_fallible(writer)
            .map_err(|(e, _)| e)?;
        debug_assert!(!writer.has_avail());

        Ok(())
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()> {
        if self
            .limit
            .checked_sub(offset)
            .is_none_or(|avail| avail < reader.remain())
        {
            return Err(Error::InvalidArgs);
        }

        self.writer()
            .skip(offset)
            .write_fallible(reader)
            .map_err(|(e, _)| e)?;
        debug_assert!(!reader.has_remain());

        Ok(())
    }
}

impl VmIoOnce for IoMem {
    fn read_once<T: PodOnce>(&self, offset: usize) -> Result<T> {
        self.reader().skip(offset).read_once()
    }

    fn write_once<T: PodOnce>(&self, offset: usize, new_val: &T) -> Result<()> {
        self.writer().skip(offset).write_once(new_val)
    }
}
