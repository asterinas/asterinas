// SPDX-License-Identifier: MPL-2.0

//! I/O memory and its allocator that allocates memory I/O (MMIO) to device drivers.

mod allocator;
mod util;

use core::{
    marker::PhantomData,
    ops::{Deref, Range},
};

use align_ext::AlignExt;

pub(crate) use self::allocator::IoMemAllocatorBuilder;
pub(super) use self::allocator::init;
use crate::{
    Error,
    cpu::{AtomicCpuSet, CpuSet},
    mm::{
        HasPaddr, HasSize, PAGE_SIZE, Paddr, PodOnce, VmIo, VmIoFill, VmIoOnce, VmReader, VmWriter,
        kspace::kvirt_area::KVirtArea,
        page_prop::{CachePolicy, PageFlags, PageProperty, PrivilegedPageFlags},
        tlb::{TlbFlushOp, TlbFlusher},
    },
    prelude::*,
    task::disable_preempt,
};

/// A marker type used for [`IoMem`],
/// representing that the underlying MMIO is used for security-sensitive operations.
#[derive(Clone, Debug)]
pub(crate) enum Sensitive {}

/// A marker type used for [`IoMem`],
/// representing that the underlying MMIO is used for security-insensitive operations.
#[derive(Clone, Debug)]
pub enum Insensitive {}

/// I/O memory.
#[derive(Debug, Clone)]
pub struct IoMem<SecuritySensitivity = Insensitive> {
    kvirt_area: Arc<KVirtArea>,
    // The actually used range for MMIO is `kvirt_area.start + offset..kvirt_area.start + offset + limit`
    offset: usize,
    limit: usize,
    pa: Paddr,
    cache_policy: CachePolicy,
    phantom: PhantomData<SecuritySensitivity>,
}

impl<SecuritySensitivity> IoMem<SecuritySensitivity> {
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
            cache_policy: self.cache_policy,
            phantom: PhantomData,
        }
    }

    /// Creates a new `IoMem`.
    ///
    /// # Safety
    ///
    /// 1. This function must be called after the kernel page table is activated.
    /// 2. The given physical address range must be in the I/O memory region.
    /// 3. Reading from or writing to I/O memory regions may have side effects.
    ///    If `SecuritySensitivity` is `Insensitive`, those side effects must
    ///    not cause soundness problems (e.g., they must not corrupt the kernel
    ///    memory).
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

            // SAFETY:
            //  - The range `first_page_start..last_page_end` is always page aligned.
            //  - FIXME: We currently do not limit the I/O memory allocator with the maximum GPA,
            //    so the address range may not fall in the GPA limit.
            //  - The caller guarantees that operations on the I/O memory do not have any side
            //    effects that may cause soundness problems, so the pages can safely be viewed as
            //    untyped memory.
            unsafe { crate::arch::tdx_guest::unprotect_gpa_tdvm_call(first_page_start, area_size).unwrap() };

            PrivilegedPageFlags::SHARED
        } else {
            PrivilegedPageFlags::empty()
        });
        #[cfg(not(target_arch = "x86_64"))]
        let priv_flags = PrivilegedPageFlags::empty();

        let prop = PageProperty {
            flags,
            cache,
            priv_flags,
        };

        let kva = {
            // SAFETY: The caller of `IoMem::new()` ensures that the given
            // physical address range is I/O memory, so it is safe to map.
            let kva = unsafe { KVirtArea::map_untracked_frames(area_size, 0, frames_range, prop) };

            let target_cpus = AtomicCpuSet::new(CpuSet::new_full());
            let mut flusher = TlbFlusher::new(&target_cpus, disable_preempt());
            flusher.issue_tlb_flush(TlbFlushOp::for_range(kva.range()));
            flusher.dispatch_tlb_flush();
            flusher.sync_tlb_flush();

            kva
        };

        Self {
            kvirt_area: Arc::new(kva),
            offset: range.start - first_page_start,
            limit: range.len(),
            pa: range.start,
            cache_policy: cache,
            phantom: PhantomData,
        }
    }

    /// Returns the cache policy of this `IoMem`.
    pub fn cache_policy(&self) -> CachePolicy {
        self.cache_policy
    }
}

#[cfg_attr(target_arch = "loongarch64", expect(unused))]
impl IoMem<Sensitive> {
    /// Reads a value of the `PodOnce` type at the specified offset using one
    /// non-tearing memory load.
    ///
    /// Except that the offset is specified explicitly, the semantics of this
    /// method is the same as [`VmReader::read_once`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that the offset and the read operation is valid,
    /// e.g., follows the specification when used for implementing drivers, does
    /// not cause any out-of-bounds access, and does not cause unsound side
    /// effects (e.g., corrupting the kernel memory).
    pub(crate) unsafe fn read_once<T: PodOnce>(&self, offset: usize) -> T {
        debug_assert!(
            offset
                .checked_add(size_of::<T>())
                .is_some_and(|end| end <= self.limit)
        );
        let ptr = (self.kvirt_area.deref().start() + self.offset + offset) as *const T;
        // SAFETY: The safety of the read operation's semantics is upheld by the caller.
        unsafe { crate::arch::io::io_mem::read_once(ptr) }
    }

    /// Writes a value of the `PodOnce` type at the specified offset using one
    /// non-tearing memory store.
    ///
    /// Except that the offset is specified explicitly, the semantics of this
    /// method is the same as [`VmWriter::write_once`].
    ///
    /// # Safety
    ///
    /// The caller must ensure that the offset and the write operation is valid,
    /// e.g., follows the specification when used for implementing drivers, does
    /// not cause any out-of-bounds access, and does not cause unsound side
    /// effects (e.g., corrupting the kernel memory).
    pub(crate) unsafe fn write_once<T: PodOnce>(&self, offset: usize, value: &T) {
        debug_assert!(
            offset
                .checked_add(size_of::<T>())
                .is_some_and(|end| end <= self.limit)
        );
        let ptr = (self.kvirt_area.deref().start() + self.offset + offset) as *mut T;
        // SAFETY: The safety of the write operation's semantics is upheld by the caller.
        unsafe { crate::arch::io::io_mem::write_once(ptr, *value) };
    }
}

impl IoMem<Insensitive> {
    /// Acquires an `IoMem` instance for the given range.
    ///
    /// The I/O memory cache policy is set to uncacheable by default.
    pub fn acquire(range: Range<Paddr>) -> Result<IoMem<Insensitive>> {
        Self::acquire_with_cache_policy(range, CachePolicy::Uncacheable)
    }

    /// Acquires an `IoMem` instance for the given range with the specified cache policy.
    pub fn acquire_with_cache_policy(
        range: Range<Paddr>,
        cache_policy: CachePolicy,
    ) -> Result<IoMem<Insensitive>> {
        allocator::IO_MEM_ALLOCATOR
            .get()
            .unwrap()
            .acquire(range, cache_policy)
            .ok_or(Error::AccessDenied)
    }
}

impl<S> IoMem<S> {
    fn base(&self) -> usize {
        self.kvirt_area.deref().start() + self.offset
    }

    fn check_range(&self, offset: usize, len: usize) -> Result<()> {
        if offset.checked_add(len).is_none_or(|end| end > self.limit) {
            return Err(Error::InvalidArgs);
        }
        Ok(())
    }

    /// Reads bytes from MMIO into the provided buffer.
    pub fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        self.check_range(offset, buf.len())?;
        let src = (self.base() + offset) as *const u8;
        let dst = buf.as_mut_ptr();

        // SAFETY: check_range guarantees a valid MMIO range; buf is a valid slice.
        unsafe {
            crate::io::io_mem::util::copy_from(dst, src, buf.len());
        }
        Ok(())
    }

    /// Writes bytes from the provided buffer into MMIO.
    pub fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        self.check_range(offset, buf.len())?;
        let src = buf.as_ptr();
        let dst = (self.base() + offset) as *mut u8;

        // SAFETY: Same as above.
        unsafe {
            crate::io::io_mem::util::copy_to(dst, src, buf.len());
        }
        Ok(())
    }
}

impl<S> VmIoOnce for IoMem<S> {
    fn read_once<T: PodOnce>(&self, offset: usize) -> Result<T> {
        self.check_range(offset, core::mem::size_of::<T>())?;
        let ptr = (self.base() + offset) as *const T;
        Ok(unsafe { crate::arch::io::io_mem::read_once(ptr) })
    }

    fn write_once<T: PodOnce>(&self, offset: usize, value: &T) -> Result<()> {
        self.check_range(offset, core::mem::size_of::<T>())?;
        let ptr = (self.base() + offset) as *mut T;
        unsafe { crate::arch::io::io_mem::write_once(ptr, *value) };
        Ok(())
    }
}

impl<S> VmIo for IoMem<S> {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()> {
        let len = writer.avail();
        self.check_range(offset, len)?;

        let src = (self.base() + offset) as *const u8;
        let dst = writer.cursor();

        // SAFETY:
        // 1. check_range guarantees a valid MMIO range.
        // 2. The writer buffer must be valid for len bytes and reside in kernel space,
        //    because copy_from uses infallible raw pointer writes.
        unsafe {
            crate::io::io_mem::util::copy_from(dst, src, len);
            writer.skip(len);
        }
        Ok(())
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()> {
        let len = reader.remain();
        self.check_range(offset, len)?;

        let dst = (self.base() + offset) as *mut u8;
        let src = reader.cursor();

        // SAFETY:
        // 1. check_range guarantees a valid MMIO range.
        // 2. The reader buffer must be valid for len bytes and reside in kernel space,
        //    because copy_to uses infallible raw pointer reads.
        unsafe {
            crate::io::io_mem::util::copy_to(dst, src, len);
            // Advance the reader cursor after copying.
            reader.skip(len);
        }
        Ok(())
    }
}

impl<S> VmIoFill for IoMem<S> {
    fn fill_zeros(&self, offset: usize, len: usize) -> core::result::Result<(), (Error, usize)> {
        if let Err(err) = self.check_range(offset, len) {
            return Err((err, 0));
        }

        let mut remaining = len;
        let mut ptr = (self.base() + offset) as *mut u8;
        let word_size = core::mem::size_of::<usize>();

        // Align destination to word size.
        while remaining > 0 && !(ptr as usize).is_multiple_of(word_size) {
            // SAFETY: check_range ensures MMIO address is valid for the range.
            unsafe { crate::arch::io::io_mem::write_once(ptr, 0u8) };
            ptr = unsafe { ptr.add(1) };
            remaining -= 1;
        }

        while remaining >= word_size {
            // SAFETY: check_range ensures MMIO address is valid for the range.
            unsafe { crate::arch::io::io_mem::write_once(ptr as *mut usize, 0usize) };
            ptr = unsafe { ptr.add(word_size) };
            remaining -= word_size;
        }

        while remaining > 0 {
            // SAFETY: check_range ensures MMIO address is valid for the range.
            unsafe { crate::arch::io::io_mem::write_once(ptr, 0u8) };
            ptr = unsafe { ptr.add(1) };
            remaining -= 1;
        }

        Ok(())
    }
}

impl<S> VmIo for &IoMem<S> {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()> {
        (**self).read(offset, writer)
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()> {
        (**self).write(offset, reader)
    }
}

impl<S> VmIo for &mut IoMem<S> {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()> {
        (**self).read(offset, writer)
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()> {
        (**self).write(offset, reader)
    }
}

impl<S> VmIoOnce for &IoMem<S> {
    fn read_once<T: PodOnce>(&self, offset: usize) -> Result<T> {
        (**self).read_once(offset)
    }

    fn write_once<T: PodOnce>(&self, offset: usize, value: &T) -> Result<()> {
        (**self).write_once(offset, value)
    }
}

impl<S> VmIoOnce for &mut IoMem<S> {
    fn read_once<T: PodOnce>(&self, offset: usize) -> Result<T> {
        (**self).read_once(offset)
    }

    fn write_once<T: PodOnce>(&self, offset: usize, value: &T) -> Result<()> {
        (**self).write_once(offset, value)
    }
}

impl<S> VmIoFill for &IoMem<S> {
    fn fill_zeros(&self, offset: usize, len: usize) -> core::result::Result<(), (Error, usize)> {
        (**self).fill_zeros(offset, len)
    }
}

impl<S> VmIoFill for &mut IoMem<S> {
    fn fill_zeros(&self, offset: usize, len: usize) -> core::result::Result<(), (Error, usize)> {
        (**self).fill_zeros(offset, len)
    }
}

impl<SecuritySensitivity> HasPaddr for IoMem<SecuritySensitivity> {
    fn paddr(&self) -> Paddr {
        self.pa
    }
}

impl<SecuritySensitivity> HasSize for IoMem<SecuritySensitivity> {
    fn size(&self) -> usize {
        self.limit
    }
}

impl<SecuritySensitivity> Drop for IoMem<SecuritySensitivity> {
    fn drop(&mut self) {
        // TODO: Multiple `IoMem` instances should not overlap, we should refactor the driver code and
        // remove the `Clone` and `IoMem::slice`. After refactoring, the `Drop` can be implemented to recycle
        // the `IoMem`.
    }
}
