// SPDX-License-Identifier: MPL-2.0

//! I/O memory and its allocator that allocates memory I/O (MMIO) to device drivers.

mod allocator;

use core::ops::{Deref, Range};

use align_ext::AlignExt;

pub(super) use self::allocator::init;
pub(crate) use self::allocator::IoMemAllocatorBuilder;
use crate::{
    mm::{
        kspace::kvirt_area::KVirtArea,
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
    kvirt_area: Arc<KVirtArea>,
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
    /// Acquires an `IoMem` instance for the given range.
    pub fn acquire(range: Range<Paddr>) -> Result<IoMem> {
        allocator::IO_MEM_ALLOCATOR
            .get()
            .unwrap()
            .acquire(range)
            .ok_or(Error::AccessDenied)
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

        let frames_range = first_page_start..last_page_end;
        let area_size = frames_range.len();

        #[cfg(target_arch = "x86_64")]
        let priv_flags = crate::arch::if_tdx_enabled!({
            assert!(
                first_page_start == range.start && last_page_end == range.end,
                "I/O memory is not page aligned, which cannot be unprotected in TDX: {:#x?}..{:#x?}",
                range.start,
                range.end,
            );

            let num_pages = area_size / PAGE_SIZE;
            // SAFETY:
            //  - The range `first_page_start..last_page_end` is always page aligned.
            //  - FIXME: We currently do not limit the I/O memory allocator with the maximum GPA,
            //    so the address range may not fall in the GPA limit.
            //  - FIXME: The I/O memory can be at a high address, so it may not be contained in the
            //    linear mapping.
            //  - The caller guarantees that operations on the I/O memory do not have any side
            //    effects that may cause soundness problems, so the pages can safely be viewed as
            //    untyped memory.
            unsafe { crate::arch::tdx_guest::unprotect_gpa_range(first_page_start, num_pages).unwrap() };

            PrivilegedPageFlags::SHARED
        } else {
            PrivilegedPageFlags::empty()
        });
        #[cfg(not(target_arch = "x86_64"))]
        let priv_flags = PrivilegedPageFlags::empty();

        let prop = PageProperty {
            has_map: true,
            flags,
            cache,
            priv_flags,
        };

        // SAFETY: The caller of `IoMem::new()` ensures that the given
        // physical address range is I/O memory, so it is safe to map.
        let kva = unsafe { KVirtArea::map_untracked_frames(area_size, 0, frames_range, prop) };

        Self {
            kvirt_area: Arc::new(kva),
            offset: range.start - first_page_start,
            limit: range.len(),
            pa: range.start,
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

impl Drop for IoMem {
    fn drop(&mut self) {
        // TODO: Multiple `IoMem` instances should not overlap, we should refactor the driver code and
        // remove the `Clone` and `IoMem::slice`. After refactoring, the `Drop` can be implemented to recycle
        // the `IoMem`.
    }
}
