// SPDX-License-Identifier: MPL-2.0

//! I/O memory.

use core::ops::{Deref, Range};

use align_ext::AlignExt;
use cfg_if::cfg_if;

use crate::{
    mm::{
        kspace::kvirt_area::{KVirtArea, Untracked},
        page_prop::{CachePolicy, PageFlags, PageProperty, PrivilegedPageFlags},
        FallibleVmRead, FallibleVmWrite, HasPaddr, Infallible, Paddr, PodOnce, VmIo, VmIoOnce,
        VmReader, VmWriter, PAGE_SIZE,
    },
    prelude::*,
    Error,
};

/// I/O memory.
#[derive(Debug, Clone)]
pub struct IoMem {
    kvirt_area: Arc<KVirtArea<Untracked>>,
    // The actually used range for MMIO is `kvirt_area.start + offset..kvirt_area.start + offset + limit`
    offset: usize,
    limit: usize,
    pa: Paddr,
}

impl HasPaddr for IoMem {
    fn paddr(&self) -> Paddr {
        self.pa
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
    pub(crate) unsafe fn new(range: Range<Paddr>, flags: PageFlags, cache: CachePolicy) -> Self {
        let first_page_start = range.start.align_down(PAGE_SIZE);
        let last_page_end = range.end.align_up(PAGE_SIZE);
        let mut new_kvirt_area = KVirtArea::<Untracked>::new(last_page_end - first_page_start);

        cfg_if! {
            if #[cfg(all(feature = "cvm_guest", target_arch = "x86_64"))] {
                let priv_flags = if tdx_guest::tdx_is_enabled() {
                    PrivilegedPageFlags::SHARED
                } else {
                    PrivilegedPageFlags::empty()
                };
            } else {
                let priv_flags = PrivilegedPageFlags::empty();
            }
        }

        let prop = PageProperty {
            flags,
            cache,
            priv_flags,
        };

        // SAFETY: The caller of `IoMem::new()` and the constructor of `new_kvirt_area` has ensured the
        // safety of this mapping.
        unsafe {
            new_kvirt_area.map_untracked_pages(
                new_kvirt_area.range(),
                first_page_start..last_page_end,
                prop,
            );
        }

        Self {
            kvirt_area: Arc::new(new_kvirt_area),
            offset: range.start - first_page_start,
            limit: range.len(),
            pa: range.start,
        }
    }

    /// Returns the physical address of the I/O memory.
    pub fn paddr(&self) -> Paddr {
        self.pa
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
            kvirt_area: self.kvirt_area.clone(),
            offset: self.offset + range.start,
            limit: range.len(),
            pa: self.pa + range.start,
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
        // SAFETY: The constructor of the `IoMem` structure has already ensured the
        // safety of reading from the mapped physical address, and the mapping is valid.
        unsafe {
            VmReader::from_kernel_space(
                (self.kvirt_area.deref().start() + self.offset) as *mut u8,
                self.limit,
            )
        }
    }

    fn writer(&self) -> VmWriter<'_, Infallible> {
        // SAFETY: The constructor of the `IoMem` structure has already ensured the
        // safety of writing to the mapped physical address, and the mapping is valid.
        unsafe {
            VmWriter::from_kernel_space(
                (self.kvirt_area.deref().start() + self.offset) as *mut u8,
                self.limit,
            )
        }
    }
}

impl VmIo for IoMem {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()> {
        let offset = offset + self.offset;
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
        let offset = offset + self.offset;
        if self
            .limit
            .checked_sub(offset)
            .is_none_or(|remain| remain < reader.remain())
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
