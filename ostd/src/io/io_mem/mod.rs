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
#[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
use crate::arch::{if_tdx_enabled, tdx_guest::unprotect_gpa_tdvm_call};
use crate::{
    Error,
    arch::io::io_mem::{read_once, write_once},
    cpu::{AtomicCpuSet, CpuSet},
    mm::{
        Fallible, HasPaddr, HasSize, Infallible, PAGE_SIZE, Paddr, PodOnce, VmIo, VmIoFill,
        VmIoOnce, VmReader, VmWriter,
        io::{
            Io,
            copy::{memcpy, memset},
        },
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
#[derive(Clone, Debug)]
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
        let priv_flags = if_tdx_enabled!({
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
            unsafe { unprotect_gpa_tdvm_call(first_page_start, area_size).unwrap() };

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
        unsafe { read_once(ptr) }
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
        unsafe { write_once(ptr, *value) };
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

    /// Reads from MMIO into fallible memory and returns the copied length.
    ///
    /// This method performs the same low-level copy primitive as [`VmIo::read`],
    /// but exposes partial progress instead of enforcing no-short-read semantics.
    pub fn read_fallible(
        &self,
        offset: usize,
        writer: &mut VmWriter,
    ) -> core::result::Result<usize, (Error, usize)> {
        let len = writer.avail();
        self.check_range(offset, len).map_err(|err| (err, 0))?;

        let src = (self.base() + offset) as *const u8;
        // SAFETY: `src` points to a validated MMIO range and `writer.cursor()` points to
        // fallible destination memory tracked by `writer`.
        let copied = unsafe { memcpy::<Fallible, Io>(writer.cursor(), src, len) };
        writer.skip(copied);

        if copied < len {
            Err((Error::PageFault, copied))
        } else {
            Ok(copied)
        }
    }

    /// Writes from fallible memory to MMIO and returns the copied length.
    ///
    /// This method performs the same low-level copy primitive as [`VmIo::write`],
    /// but exposes partial progress instead of enforcing no-short-write semantics.
    pub fn write_fallible(
        &self,
        offset: usize,
        reader: &mut VmReader,
    ) -> core::result::Result<usize, (Error, usize)> {
        let len = reader.remain();
        self.check_range(offset, len).map_err(|err| (err, 0))?;

        let dst = (self.base() + offset) as *mut u8;
        // SAFETY: `dst` points to a validated MMIO range and `reader.cursor()` points to
        // fallible source memory tracked by `reader`.
        let copied = unsafe { memcpy::<Io, Fallible>(dst, reader.cursor(), len) };
        reader.skip(copied);

        if copied < len {
            Err((Error::PageFault, copied))
        } else {
            Ok(copied)
        }
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
        let val = unsafe { read_once(ptr) };
        Ok(val)
    }

    fn write_once<T: PodOnce>(&self, offset: usize, value: &T) -> Result<()> {
        self.check_range(offset, size_of::<T>())?;
        let ptr = (self.base() + offset) as *mut T;
        if !ptr.is_aligned() {
            return Err(Error::InvalidArgs);
        }

        // SAFETY: The pointer is properly aligned and within the validated range.
        unsafe { write_once(ptr, *value) };
        Ok(())
    }
}

impl VmIo for IoMem<Insensitive> {
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()> {
        let len = writer.avail();
        self.check_range(offset, len)?;

        let src = (self.base() + offset) as *const u8;
        // SAFETY: `src` points to a validated MMIO range and `writer.cursor()` points to
        // fallible destination memory tracked by `writer`.
        let copied = unsafe { memcpy::<Fallible, Io>(writer.cursor(), src, len) };
        if copied < len {
            return Err(Error::PageFault);
        }

        writer.skip(copied);
        Ok(())
    }

    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        let len = buf.len();
        self.check_range(offset, len)?;
        let src = (self.base() + offset) as *const u8;
        let dst = buf.as_mut_ptr();

        // SAFETY: The `dst` and `src` buffers are valid to write and read for `len` bytes.
        unsafe { memcpy::<Infallible, Io>(dst, src, len) };
        Ok(())
    }

    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()> {
        let len = reader.remain();
        self.check_range(offset, len)?;

        let dst = (self.base() + offset) as *mut u8;
        // SAFETY: `dst` points to a validated MMIO range and `reader.cursor()` points to
        // fallible source memory tracked by `reader`.
        let copied = unsafe { memcpy::<Io, Fallible>(dst, reader.cursor(), len) };
        if copied < len {
            return Err(Error::PageFault);
        }

        reader.skip(copied);
        Ok(())
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        let len = buf.len();
        self.check_range(offset, len)?;
        let src = buf.as_ptr();
        let dst = (self.base() + offset) as *mut u8;

        // SAFETY: The `dst` and `src` buffers are valid to write and read for `len` bytes.
        unsafe { memcpy::<Io, Infallible>(dst, src, len) };
        Ok(())
    }
}

impl VmIoFill for IoMem<Insensitive> {
    fn fill_zeros(&self, offset: usize, len: usize) -> core::result::Result<(), (Error, usize)> {
        if len == 0 {
            return Ok(());
        }

        if offset > self.limit {
            return Err((Error::InvalidArgs, 0));
        }

        let available = self.limit - offset;
        let write_len = core::cmp::min(len, available);
        if write_len == 0 {
            return Err((Error::InvalidArgs, 0));
        }

        let dst = (self.base() + offset) as *mut u8;
        // SAFETY: `dst` points to the validated MMIO subrange of `write_len` bytes.
        unsafe { memset::<Io>(dst, 0u8, write_len) };

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

#[cfg(ktest)]
mod test {
    use core::mem::size_of;

    use crate::{
        arch::io::io_mem::{copy_from_mmio, copy_to_mmio, read_once, write_once},
        prelude::ktest,
    };

    #[ktest]
    fn read_write_u8() {
        let mut data: u8 = 0;
        // SAFETY: `data` is valid for a single MMIO read/write.
        unsafe {
            write_once(&mut data, 42u8);
            assert_eq!(read_once(&data), 42u8);
        }
    }

    #[ktest]
    fn read_write_u16() {
        let mut data: u16 = 0;
        let val: u16 = 0x1234;
        // SAFETY: `data` is valid for a single MMIO read/write.
        unsafe {
            write_once(&mut data, val);
            assert_eq!(read_once(&data), val);
        }
    }

    #[ktest]
    fn read_write_u32() {
        let mut data: u32 = 0;
        let val: u32 = 0x12345678;
        // SAFETY: `data` is valid for a single MMIO read/write.
        unsafe {
            write_once(&mut data, val);
            assert_eq!(read_once(&data), val);
        }
    }

    #[ktest]
    fn read_write_u64() {
        let mut data: u64 = 0;
        let val: u64 = 0xDEADBEEFCAFEBABE;
        // SAFETY: `data` is valid for a single MMIO read/write.
        unsafe {
            write_once(&mut data, val);
            assert_eq!(read_once(&data), val);
        }
    }

    #[ktest]
    fn boundary_overlap() {
        let mut data: [u8; 2] = [0xAA, 0xBB];
        // SAFETY: `data` is valid for a single MMIO read/write.
        unsafe {
            write_once(&mut data[0], 0x11u8);
            assert_eq!(data[0], 0x11);
            assert_eq!(data[1], 0xBB);
        }
    }

    fn fill_pattern(buf: &mut [u8]) {
        for (idx, byte) in buf.iter_mut().enumerate() {
            *byte = (idx as u8).wrapping_mul(3).wrapping_add(1);
        }
    }

    fn run_copy_from_case(src_offset: usize, dst_offset: usize, len: usize) {
        let mut src = [0u8; 64];
        let mut dst = [0u8; 64];
        fill_pattern(&mut src);

        // SAFETY: Offsets are validated by callers before this helper is invoked.
        let src_ptr = unsafe { src.as_ptr().add(src_offset) };
        // SAFETY: Offsets are validated by callers before this helper is invoked.
        let dst_ptr = unsafe { dst.as_mut_ptr().add(dst_offset) };

        // SAFETY: The test buffers are valid for the requested range.
        unsafe { copy_from_mmio(dst_ptr, src_ptr, len) };

        assert_eq!(
            &dst[dst_offset..dst_offset + len],
            &src[src_offset..src_offset + len]
        );
    }

    fn run_copy_to_case(src_offset: usize, dst_offset: usize, len: usize) {
        let mut src = [0u8; 64];
        let mut dst = [0u8; 64];
        fill_pattern(&mut src);

        // SAFETY: Offsets are validated by callers before this helper is invoked.
        let src_ptr = unsafe { src.as_ptr().add(src_offset) };
        // SAFETY: Offsets are validated by callers before this helper is invoked.
        let dst_ptr = unsafe { dst.as_mut_ptr().add(dst_offset) };

        // SAFETY: The test buffers are valid for the requested range.
        unsafe { copy_to_mmio(src_ptr, dst_ptr, len) };

        assert_eq!(
            &dst[dst_offset..dst_offset + len],
            &src[src_offset..src_offset + len]
        );
    }

    #[ktest]
    fn copy_from_alignment_and_sizes() {
        let word_size = size_of::<usize>();
        let sizes = [
            0,
            1,
            word_size.saturating_sub(1),
            word_size,
            word_size + 1,
            word_size * 2 + 3,
        ];
        let offsets = [0, 1, 2];

        for &len in &sizes {
            for &src_offset in &offsets {
                for &dst_offset in &offsets {
                    if src_offset + len <= 64 && dst_offset + len <= 64 {
                        run_copy_from_case(src_offset, dst_offset, len);
                    }
                }
            }
        }
    }

    #[ktest]
    fn copy_to_alignment_and_sizes() {
        let word_size = size_of::<usize>();
        let sizes = [
            0,
            1,
            word_size.saturating_sub(1),
            word_size,
            word_size + 1,
            word_size * 2 + 3,
        ];
        let offsets = [0, 1, 2];

        for &len in &sizes {
            for &src_offset in &offsets {
                for &dst_offset in &offsets {
                    if src_offset + len <= 64 && dst_offset + len <= 64 {
                        run_copy_to_case(src_offset, dst_offset, len);
                    }
                }
            }
        }
    }
}
