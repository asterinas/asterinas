// SPDX-License-Identifier: MPL-2.0

//! I/O memory and its allocator that allocates memory I/O (MMIO) to device drivers.

mod allocator;
pub(crate) mod util;

use core::{
    marker::PhantomData,
    ops::{Deref, Range},
};

use align_ext::AlignExt;
use inherit_methods_macro::inherit_methods;

pub(crate) use self::allocator::IoMemAllocatorBuilder;
pub(super) use self::allocator::init;
use crate::{
    Error,
    cpu::{AtomicCpuSet, CpuSet},
    io::io_mem::util::{
        copy_from_io_mem, copy_from_io_to_writer, copy_from_reader_to_io, copy_to_io_mem,
    },
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

    /// Returns the base virtual address of the MMIO range.
    fn base(&self) -> usize {
        self.kvirt_area.deref().start() + self.offset
    }

    /// Validates that the offset range lies within the MMIO window.
    fn check_range(&self, offset: usize, len: usize) -> Result<()> {
        if offset.checked_add(len).is_none_or(|end| end > self.limit) {
            return Err(Error::InvalidArgs);
        }
        Ok(())
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
        debug_assert!(offset + size_of::<T>() <= self.limit);
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
        debug_assert!(offset + size_of::<T>() <= self.limit);
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

impl VmIoOnce for IoMem<Insensitive> {
    fn read_once<T: PodOnce>(&self, offset: usize) -> Result<T> {
        self.check_range(offset, size_of::<T>())?;
        let ptr = (self.base() + offset) as *const T;
        if !ptr.is_aligned() {
            return Err(Error::InvalidArgs);
        }

        // SAFETY: The pointer is properly aligned and within the validated range.
        let val = unsafe { crate::arch::io::io_mem::read_once(ptr) };
        Ok(val)
    }

    fn write_once<T: PodOnce>(&self, offset: usize, value: &T) -> Result<()> {
        self.check_range(offset, size_of::<T>())?;
        let ptr = (self.base() + offset) as *mut T;
        if !ptr.is_aligned() {
            return Err(Error::InvalidArgs);
        }

        // SAFETY: The pointer is properly aligned and within the validated range.
        unsafe { crate::arch::io::io_mem::write_once(ptr, *value) };
        Ok(())
    }
}

impl VmIo for IoMem<Insensitive> {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()> {
        let len = writer.avail();
        self.check_range(offset, len)?;

        let src = (self.base() + offset) as *const u8;

        // SAFETY: `check_range` guarantees a valid MMIO range for `len` bytes.
        unsafe { copy_from_io_to_writer(writer, src, len) }.map_err(|(err, _)| err)
    }

    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        let len = buf.len();
        self.check_range(offset, len)?;
        let src = (self.base() + offset) as *const u8;
        let dst = buf.as_mut_ptr();

        // SAFETY: The `dst` and `src` buffers are valid to write and read for `len` bytes.
        unsafe { copy_from_io_mem(dst, src, len) };
        Ok(())
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()> {
        let len = reader.remain();
        self.check_range(offset, len)?;

        let dst = (self.base() + offset) as *mut u8;

        // SAFETY: `check_range` guarantees a valid MMIO range for `len` bytes.
        unsafe { copy_from_reader_to_io(reader, dst, len) }.map_err(|(err, _)| err)
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        let len = buf.len();
        self.check_range(offset, len)?;
        let src = buf.as_ptr();
        let dst = (self.base() + offset) as *mut u8;

        // SAFETY: The `dst` and `src` buffers are valid to write and read for `len` bytes.
        unsafe { copy_to_io_mem(src, dst, len) };
        Ok(())
    }
}

impl VmIoFill for IoMem<Insensitive> {
    fn fill_zeros(&self, offset: usize, len: usize) -> core::result::Result<(), (Error, usize)> {
        if offset > self.limit {
            return Err((Error::InvalidArgs, 0));
        }

        let available = self.limit - offset;
        let write_len = core::cmp::min(len, available);
        if write_len == 0 {
            return Ok(());
        }

        let mut remaining = write_len;
        let mut ptr = (self.base() + offset) as *mut u8;
        let word_size = size_of::<usize>();

        // Align destination to word size.
        while remaining >= 1 && (ptr.addr() & (word_size - 1)) != 0 {
            // SAFETY: `check_range` guarantees a valid MMIO range for the range.
            unsafe { crate::arch::io::io_mem::write_once(ptr, 0u8) };
            ptr = ptr.wrapping_add(1);
            remaining -= 1;
        }

        while remaining >= word_size {
            // SAFETY: `check_range` guarantees a valid MMIO range for the range.
            unsafe { crate::arch::io::io_mem::write_once(ptr.cast::<usize>(), 0usize) };
            ptr = ptr.wrapping_add(word_size);
            remaining -= word_size;
        }

        while remaining >= 1 {
            // SAFETY: The remaining range is within the validated MMIO window.
            unsafe { crate::arch::io::io_mem::write_once(ptr, 0u8) };
            ptr = ptr.wrapping_add(1);
            remaining -= 1;
        }

        if write_len == len {
            Ok(())
        } else {
            Err((Error::InvalidArgs, write_len))
        }
    }
}

macro_rules! impl_vm_io_pointer {
    ($ty:ty, $from:tt) => {
        #[inherit_methods(from = $from)]
        impl VmIo for $ty {
            fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()>;
            fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()>;
        }

        #[inherit_methods(from = $from)]
        impl VmIoOnce for $ty {
            fn read_once<T: PodOnce>(&self, offset: usize) -> Result<T>;
            fn write_once<T: PodOnce>(&self, offset: usize, value: &T) -> Result<()>;
        }

        #[inherit_methods(from = $from)]
        impl VmIoFill for $ty {
            fn fill_zeros(
                &self,
                offset: usize,
                len: usize,
            ) -> core::result::Result<(), (Error, usize)>;
        }
    };
}

impl_vm_io_pointer!(&IoMem<Insensitive>, "(**self)");
impl_vm_io_pointer!(&mut IoMem<Insensitive>, "(**self)");

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
