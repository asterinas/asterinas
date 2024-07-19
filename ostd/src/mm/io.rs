// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use core::marker::PhantomData;

use align_ext::AlignExt;
use inherit_methods_macro::inherit_methods;

use crate::{
    arch::mm::__memcpy_fallible,
    mm::{
        kspace::{KERNEL_BASE_VADDR, KERNEL_END_VADDR},
        MAX_USERSPACE_VADDR,
    },
    prelude::*,
    Error, Pod,
};

/// A trait that enables reading/writing data from/to a VM object,
/// e.g., [`Segment`], [`Vec<Frame>`] and [`Frame`].
///
/// # Concurrency
///
/// The methods may be executed by multiple concurrent reader and writer
/// threads. In this case, if the results of concurrent reads or writes
/// desire predictability or atomicity, the users should add extra mechanism
/// for such properties.
///
/// [`Segment`]: crate::mm::Segment
/// [`Frame`]: crate::mm::Frame
pub trait VmIo: Send + Sync {
    /// Reads a specified number of bytes at a specified offset into a given buffer.
    ///
    /// # No short reads
    ///
    /// On success, the output `buf` must be filled with the requested data
    /// completely. If, for any reason, the requested data is only partially
    /// available, then the method shall return an error.
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()>;

    /// Reads a value of a specified type at a specified offset.
    fn read_val<T: Pod>(&self, offset: usize) -> Result<T> {
        let mut val = T::new_uninit();
        self.read_bytes(offset, val.as_bytes_mut())?;
        Ok(val)
    }

    /// Reads a slice of a specified type at a specified offset.
    ///
    /// # No short reads
    ///
    /// Similar to [`read_bytes`].
    ///
    /// [`read_bytes`]: VmIo::read_bytes
    fn read_slice<T: Pod>(&self, offset: usize, slice: &mut [T]) -> Result<()> {
        let len_in_bytes = core::mem::size_of_val(slice);
        let ptr = slice as *mut [T] as *mut u8;
        // SAFETY: the slice can be transmuted to a writable byte slice since the elements
        // are all Plain-Old-Data (Pod) types.
        let buf = unsafe { core::slice::from_raw_parts_mut(ptr, len_in_bytes) };
        self.read_bytes(offset, buf)
    }

    /// Writes a specified number of bytes from a given buffer at a specified offset.
    ///
    /// # No short writes
    ///
    /// On success, the input `buf` must be written to the VM object entirely.
    /// If, for any reason, the input data can only be written partially,
    /// then the method shall return an error.
    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()>;

    /// Writes a value of a specified type at a specified offset.
    fn write_val<T: Pod>(&self, offset: usize, new_val: &T) -> Result<()> {
        self.write_bytes(offset, new_val.as_bytes())?;
        Ok(())
    }

    /// Writes a slice of a specified type at a specified offset.
    ///
    /// # No short write
    ///
    /// Similar to [`write_bytes`].
    ///
    /// [`write_bytes`]: VmIo::write_bytes
    fn write_slice<T: Pod>(&self, offset: usize, slice: &[T]) -> Result<()> {
        let len_in_bytes = core::mem::size_of_val(slice);
        let ptr = slice as *const [T] as *const u8;
        // SAFETY: the slice can be transmuted to a readable byte slice since the elements
        // are all Plain-Old-Data (Pod) types.
        let buf = unsafe { core::slice::from_raw_parts(ptr, len_in_bytes) };
        self.write_bytes(offset, buf)
    }

    /// Writes a sequence of values given by an iterator (`iter`) from the specified offset (`offset`).
    ///
    /// The write process stops until the VM object does not have enough remaining space
    /// or the iterator returns `None`. If any value is written, the function returns `Ok(nr_written)`,
    /// where `nr_written` is the number of the written values.
    ///
    /// The offset of every value written by this method is aligned to the `align`-byte boundary.
    /// Naturally, when `align` equals to `0` or `1`, then the argument takes no effect:
    /// the values will be written in the most compact way.
    ///
    /// # Example
    ///
    /// Initializes an VM object with the same value can be done easily with `write_values`.
    ///
    /// ```
    /// use core::iter::self;
    ///
    /// let _nr_values = vm_obj.write_vals(0, iter::repeat(0_u32), 0).unwrap();
    /// ```
    ///
    /// # Panics
    ///
    /// This method panics if `align` is greater than two,
    /// but not a power of two, in release mode.
    fn write_vals<'a, T: Pod + 'a, I: Iterator<Item = &'a T>>(
        &self,
        offset: usize,
        iter: I,
        align: usize,
    ) -> Result<usize> {
        let mut nr_written = 0;

        let (mut offset, item_size) = if (align >> 1) == 0 {
            // align is 0 or 1
            (offset, core::mem::size_of::<T>())
        } else {
            // align is more than 2
            (
                offset.align_up(align),
                core::mem::size_of::<T>().align_up(align),
            )
        };

        for item in iter {
            match self.write_val(offset, item) {
                Ok(_) => {
                    offset += item_size;
                    nr_written += 1;
                }
                Err(e) => {
                    if nr_written > 0 {
                        return Ok(nr_written);
                    }
                    return Err(e);
                }
            }
        }

        Ok(nr_written)
    }
}

macro_rules! impl_vmio_pointer {
    ($typ:ty,$from:tt) => {
        #[inherit_methods(from = $from)]
        impl<T: VmIo> VmIo for $typ {
            fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()>;
            fn read_val<F: Pod>(&self, offset: usize) -> Result<F>;
            fn read_slice<F: Pod>(&self, offset: usize, slice: &mut [F]) -> Result<()>;
            fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()>;
            fn write_val<F: Pod>(&self, offset: usize, new_val: &F) -> Result<()>;
            fn write_slice<F: Pod>(&self, offset: usize, slice: &[F]) -> Result<()>;
        }
    };
}

impl_vmio_pointer!(&T, "(**self)");
impl_vmio_pointer!(&mut T, "(**self)");
impl_vmio_pointer!(Box<T>, "(**self)");
impl_vmio_pointer!(Arc<T>, "(**self)");

/// A marker structure used for [`VmReader`] and [`VmWriter`],
/// representing their operated memory scope is in user space.
pub struct UserSpace;

/// A marker structure used for [`VmReader`] and [`VmWriter`],
/// representing their operated memory scope is in kernel space.
pub struct KernelSpace;

/// Copies `len` bytes from `src` to `dst`.
///
/// # Safety
///
/// - Mappings of virtual memory range [`src`..`src` + len] and [`dst`..`dst` + len]
///   must be [valid].
/// - If one of the memory represents typed memory, these two virtual
///   memory ranges and their corresponding physical pages should _not_ overlap.
///
/// Operation on typed memory may be safe only if it is plain-old-data. Otherwise,
/// the safety requirements of [`core::ptr::copy`] should also be considered,
/// except for the requirement that no concurrent access is allowed.
///
/// [valid]: core::ptr#safety
unsafe fn memcpy(dst: *mut u8, src: *const u8, len: usize) {
    // The safety conditions of this method explicitly allow data races on untyped memory because
    // this method can be used to copy data to/from a page that is also mapped to user space, so
    // avoiding data races is not feasible in this case.
    //
    // This method is implemented by calling `volatile_copy_memory`. Note that even with the
    // "volatile" keyword, data races are still considered undefined behavior (UB) in both the Rust
    // documentation and the C/C++ standards. In general, UB makes the behavior of the entire
    // program unpredictable, usually due to compiler optimizations that assume the absence of UB.
    // However, in this particular case, considering that the Linux kernel uses the "volatile"
    // keyword to implement `READ_ONCE` and `WRITE_ONCE`, the compiler is extremely unlikely to
    // break our code unless it also breaks the Linux kernel.
    //
    // For more details and future possibilities, see
    // <https://github.com/asterinas/asterinas/pull/1001#discussion_r1667317406>.
    core::intrinsics::volatile_copy_memory(dst, src, len);
}

/// Copies `len` bytes from `src` to `dst`.
/// This function will early stop copying if encountering an unresolvable page fault.
///
/// Returns the number of successfully copied bytes.
///
/// # Safety
///
/// - Users should ensure one of [`src`..`src` + len] and [`dst`..`dst` + len]
///   is in user space, and the other virtual memory range is in kernel space
///   and is ensured to be [valid].
/// - Users should ensure this function only be invoked when a suitable page
///   table is activated.
/// - The underlying physical memory range of [`src`..`src` + len] and [`dst`..`dst` + len]
///   should _not_ overlap if the kernel space memory represent typed memory.
///
/// [valid]: core::ptr#safety
unsafe fn memcpy_fallible(dst: *mut u8, src: *const u8, len: usize) -> usize {
    let failed_bytes = __memcpy_fallible(dst, src, len);
    len - failed_bytes
}

/// `VmReader` is a reader for reading data from a contiguous range of memory.
///
/// The memory range read by `VmReader` can be in either kernel space or user space.
/// When the operating range is in kernel space, the memory within that range
/// is guaranteed to be valid.
/// When the operating range is in user space, it is ensured that the page table of
/// the process creating the `VmReader` is active for the duration of `'a`.
///
/// When perform reading with a `VmWriter`, if one of them represents typed memory,
/// it can ensure that the reading range in this reader and writing range in the
/// writer are not overlapped.
///
/// NOTE: The overlap mentioned above is at both the virtual address level
/// and physical address level. There is not guarantee for the operation results
/// of `VmReader` and `VmWriter` in overlapping untyped addresses, and it is
/// the user's responsibility to handle this situation.
pub struct VmReader<'a, Space = KernelSpace> {
    cursor: *const u8,
    end: *const u8,
    phantom: PhantomData<(&'a [u8], Space)>,
}

macro_rules! impl_read_fallible {
    ($read_space:ty, $write_space:ty) => {
        impl<'a> VmReader<'a, $read_space> {
            /// Reads all data into the writer until one of the three conditions is met:
            /// 1. The reader has no remaining data.
            /// 2. The writer has no available space.
            /// 3. The reader/writer encounters some error.
            ///
            /// On success, the number of bytes read is returned;
            /// On error, both the error and the number of bytes read so far are returned.
            pub fn read_fallible(
                &mut self,
                writer: &mut VmWriter<'_, $write_space>,
            ) -> core::result::Result<usize, (Error, usize)> {
                let copy_len = self.remain().min(writer.avail());
                if copy_len == 0 {
                    return Ok(0);
                }

                // SAFETY: This method is only implemented when one of the operated
                // `VmReader` or `VmWriter` is in user space.
                // The the corresponding page table of the user space memory is
                // guaranteed to be activated due to its construction requirement.
                // The kernel space memory range will be valid since `copy_len` is the minimum
                // of the reader's remaining data and the writer's available space, and will
                // not overlap with user space memory range in physical address level if it
                // represents typed memory.
                let copied_len = unsafe {
                    let copied_len = memcpy_fallible(writer.cursor, self.cursor, copy_len);
                    self.cursor = self.cursor.add(copied_len);
                    writer.cursor = writer.cursor.add(copied_len);
                    copied_len
                };
                if copied_len < copy_len {
                    Err((Error::PageFault, copied_len))
                } else {
                    Ok(copied_len)
                }
            }
        }
    };
}

macro_rules! impl_write_fallible {
    ($read_space:ty, $write_space:ty) => {
        impl<'a> VmWriter<'a, $write_space> {
            /// Writes all data from the reader until one of the three conditions is met:
            /// 1. The reader has no remaining data.
            /// 2. The writer has no available space.
            /// 3. The reader/writer encounters some error.
            ///
            /// On success, the number of bytes written is returned;
            /// On error, both the error and the number of bytes written so far are returned.
            pub fn write_fallible(
                &mut self,
                reader: &mut VmReader<'_, $read_space>,
            ) -> core::result::Result<usize, (Error, usize)> {
                reader.read_fallible(self)
            }
        }
    };
}

// TODO: implement an additional function `memcpy_nonoverlapping_fallible`
// to implement read/write instruction from user space to user space.
impl_read_fallible!(UserSpace, KernelSpace);
impl_read_fallible!(KernelSpace, UserSpace);
impl_write_fallible!(UserSpace, KernelSpace);
impl_write_fallible!(KernelSpace, UserSpace);

impl<'a> VmReader<'a, KernelSpace> {
    /// Constructs a `VmReader` from a pointer and a length, which represents
    /// a memory range in kernel space.
    ///
    /// # Safety
    ///
    /// Users must ensure the memory from `ptr` to `ptr.add(len)` is contiguous.
    /// Users must ensure the memory is valid during the entire period of `'a`.
    /// Users must ensure the memory should _not_ overlap with other `VmWriter`s
    /// with typed memory, and if the memory range in this `VmReader` is typed,
    /// it should _not_ overlap with other `VmWriter`s.
    /// The user space memory is treated as untyped.
    pub unsafe fn from_kernel_space(ptr: *const u8, len: usize) -> Self {
        // If casting a zero sized slice to a pointer, the pointer may be null
        // and does not reside in our kernel space range.
        debug_assert!(len == 0 || KERNEL_BASE_VADDR <= ptr as usize);
        debug_assert!(len == 0 || ptr.add(len) as usize <= KERNEL_END_VADDR);

        Self {
            cursor: ptr,
            end: ptr.add(len),
            phantom: PhantomData,
        }
    }

    /// Reads all data into the writer until one of the two conditions is met:
    /// 1. The reader has no remaining data.
    /// 2. The writer has no available space.
    ///
    /// Returns the number of bytes read.
    pub fn read(&mut self, writer: &mut VmWriter<'_, KernelSpace>) -> usize {
        let copy_len = self.remain().min(writer.avail());
        if copy_len == 0 {
            return 0;
        }

        // SAFETY: the reading memory range and writing memory range will be valid
        // since `copy_len` is the minimum of the reader's remaining data and the
        // writer's available space, and will not overlap if one of them represents
        // typed memory.
        unsafe {
            memcpy(writer.cursor, self.cursor, copy_len);
            self.cursor = self.cursor.add(copy_len);
            writer.cursor = writer.cursor.add(copy_len);
        }

        copy_len
    }

    /// Reads a value of `Pod` type.
    ///
    /// If the length of the `Pod` type exceeds `self.remain()`,
    /// this method will return `Err`.
    pub fn read_val<T: Pod>(&mut self) -> Result<T> {
        if self.remain() < core::mem::size_of::<T>() {
            return Err(Error::InvalidArgs);
        }

        let mut val = T::new_uninit();
        let mut writer = VmWriter::from(val.as_bytes_mut());

        self.read(&mut writer);
        Ok(val)
    }
}

impl<'a> VmReader<'a, UserSpace> {
    /// Constructs a `VmReader` from a pointer and a length, which represents
    /// a memory range in user space.
    ///
    /// # Safety
    ///
    /// Users must ensure the memory from `ptr` to `ptr.add(len)` is contiguous.
    /// Users must ensure that the page table for the process in which this constructor is called
    /// are active during the entire period of `'a`.
    pub unsafe fn from_user_space(ptr: *const u8, len: usize) -> Self {
        debug_assert!((ptr as usize).checked_add(len).unwrap_or(usize::MAX) <= MAX_USERSPACE_VADDR);

        Self {
            cursor: ptr,
            end: ptr.add(len),
            phantom: PhantomData,
        }
    }

    /// Reads a value of `Pod` type.
    ///
    /// If the length of the `Pod` type exceeds `self.remain()`,
    /// or the value can not be read completely,
    /// this method will return `Err`.
    pub fn read_val<T: Pod>(&mut self) -> Result<T> {
        if self.remain() < core::mem::size_of::<T>() {
            return Err(Error::InvalidArgs);
        }

        let mut val = T::new_uninit();
        let mut writer = VmWriter::from(val.as_bytes_mut());
        self.read_fallible(&mut writer)
            .map(|_| val)
            .map_err(|err| err.0)
    }
}

impl<'a, Space> VmReader<'a, Space> {
    /// Returns the number of bytes for the remaining data.
    pub const fn remain(&self) -> usize {
        // SAFETY: the end is equal to or greater than the cursor.
        unsafe { self.end.sub_ptr(self.cursor) }
    }

    /// Returns the cursor pointer, which refers to the address of the next byte to read.
    pub const fn cursor(&self) -> *const u8 {
        self.cursor
    }

    /// Returns if it has remaining data to read.
    pub const fn has_remain(&self) -> bool {
        self.remain() > 0
    }

    /// Limits the length of remaining data.
    ///
    /// This method ensures the post condition of `self.remain() <= max_remain`.
    pub const fn limit(mut self, max_remain: usize) -> Self {
        if max_remain < self.remain() {
            // SAFETY: the new end is less than the old end.
            unsafe { self.end = self.cursor.add(max_remain) };
        }
        self
    }

    /// Skips the first `nbytes` bytes of data.
    /// The length of remaining data is decreased accordingly.
    ///
    /// # Panic
    ///
    /// If `nbytes` is greater than `self.remain()`, then the method panics.
    pub fn skip(mut self, nbytes: usize) -> Self {
        assert!(nbytes <= self.remain());

        // SAFETY: the new cursor is less than or equal to the end.
        unsafe { self.cursor = self.cursor.add(nbytes) };
        self
    }
}

impl<'a> From<&'a [u8]> for VmReader<'a> {
    fn from(slice: &'a [u8]) -> Self {
        // SAFETY: the range of memory is contiguous and is valid during `'a`,
        // and will not overlap with other `VmWriter` since the slice already has
        // an immutable reference. The slice will not be mapped to the user space hence
        // it will also not overlap with `VmWriter` generated from user space.
        unsafe { Self::from_kernel_space(slice.as_ptr(), slice.len()) }
    }
}

/// `VmWriter` is a writer for writing data to a contiguous range of memory.
///
/// The memory range write by `VmWriter` can be in either kernel space or user space.
/// When the operating range is in kernel space, the memory within that range
/// is guaranteed to be valid.
/// When the operating range is in user space, it is ensured that the page table of
/// the process creating the `VmWriter` is active for the duration of `'a`.
///
/// When perform writing with a `VmReader`, if one of them represents typed memory,
/// it can ensure that the writing range in this writer and reading range in the
/// reader are not overlapped.
///
/// NOTE: The overlap mentioned above is at both the virtual address level
/// and physical address level. There is not guarantee for the operation results
/// of `VmReader` and `VmWriter` in overlapping untyped addresses, and it is
/// the user's responsibility to handle this situation.
pub struct VmWriter<'a, Space = KernelSpace> {
    cursor: *mut u8,
    end: *mut u8,
    phantom: PhantomData<(&'a mut [u8], Space)>,
}

impl<'a> VmWriter<'a, KernelSpace> {
    /// Constructs a `VmWriter` from a pointer and a length, which represents
    /// a memory range in kernel space.
    ///
    /// # Safety
    ///
    /// Users must ensure the memory from `ptr` to `ptr.add(len)` is contiguous.
    /// Users must ensure the memory is valid during the entire period of `'a`.
    /// Users must ensure the memory should _not_ overlap with other `VmWriter`s
    /// and `VmReader`s with typed memory, and if the memory range in this `VmWriter`
    /// is typed, it should _not_ overlap with other `VmReader`s and `VmWriter`s.
    /// The user space memory is treated as untyped.
    pub unsafe fn from_kernel_space(ptr: *mut u8, len: usize) -> Self {
        // If casting a zero sized slice to a pointer, the pointer may be null
        // and does not reside in our kernel space range.
        debug_assert!(len == 0 || KERNEL_BASE_VADDR <= ptr as usize);
        debug_assert!(len == 0 || ptr.add(len) as usize <= KERNEL_END_VADDR);

        Self {
            cursor: ptr,
            end: ptr.add(len),
            phantom: PhantomData,
        }
    }

    /// Writes all data from the reader until one of the two conditions is met:
    /// 1. The reader has no remaining data.
    /// 2. The writer has no available space.
    ///
    /// Returns the number of bytes written.
    pub fn write(&mut self, reader: &mut VmReader<'_, KernelSpace>) -> usize {
        reader.read(self)
    }

    /// Writes a value of `Pod` type.
    ///
    /// If the length of the `Pod` type exceeds `self.avail()`,
    /// this method will return `Err`.
    pub fn write_val<T: Pod>(&mut self, new_val: &T) -> Result<()> {
        if self.avail() < core::mem::size_of::<T>() {
            return Err(Error::InvalidArgs);
        }

        let mut reader = VmReader::from(new_val.as_bytes());
        self.write(&mut reader);
        Ok(())
    }

    /// Fills the available space by repeating `value`.
    ///
    /// Returns the number of values written.
    ///
    /// # Panic
    ///
    /// The size of the available space must be a multiple of the size of `value`.
    /// Otherwise, the method would panic.
    pub fn fill<T: Pod>(&mut self, value: T) -> usize {
        let avail = self.avail();

        assert!((self.cursor as *mut T).is_aligned());
        assert!(avail % core::mem::size_of::<T>() == 0);

        let written_num = avail / core::mem::size_of::<T>();

        for i in 0..written_num {
            // SAFETY: `written_num` is calculated by the avail size and the size of the type `T`,
            // hence the `add` operation and `write` operation are valid and will only manipulate
            // the memory managed by this writer.
            unsafe {
                (self.cursor as *mut T).add(i).write(value);
            }
        }

        // The available space has been filled so this cursor can be moved to the end.
        self.cursor = self.end;
        written_num
    }
}

impl<'a> VmWriter<'a, UserSpace> {
    /// Constructs a `VmWriter` from a pointer and a length, which represents
    /// a memory range in user space.
    ///
    /// # Safety
    ///
    /// Users must ensure the memory from `ptr` to `ptr.add(len)` is contiguous.
    /// Users must ensure that the page table for the process in which this constructor is called
    /// are active during the entire period of `'a`.
    pub unsafe fn from_user_space(ptr: *mut u8, len: usize) -> Self {
        debug_assert!((ptr as usize).checked_add(len).unwrap_or(usize::MAX) <= MAX_USERSPACE_VADDR);

        Self {
            cursor: ptr,
            end: ptr.add(len),
            phantom: PhantomData,
        }
    }

    /// Writes a value of `Pod` type.
    ///
    /// If the length of the `Pod` type exceeds `self.avail()`,
    /// or the value can not be write completely,
    /// this method will return `Err`.
    pub fn write_val<T: Pod>(&mut self, new_val: &T) -> Result<()> {
        if self.avail() < core::mem::size_of::<T>() {
            return Err(Error::InvalidArgs);
        }

        let mut reader = VmReader::from(new_val.as_bytes());
        self.write_fallible(&mut reader).map_err(|err| err.0)?;
        Ok(())
    }
}

impl<'a, Space> VmWriter<'a, Space> {
    /// Returns the number of bytes for the available space.
    pub const fn avail(&self) -> usize {
        // SAFETY: the end is equal to or greater than the cursor.
        unsafe { self.end.sub_ptr(self.cursor) }
    }

    /// Returns the cursor pointer, which refers to the address of the next byte to write.
    pub const fn cursor(&self) -> *mut u8 {
        self.cursor
    }

    /// Returns if it has available space to write.
    pub const fn has_avail(&self) -> bool {
        self.avail() > 0
    }

    /// Limits the length of available space.
    ///
    /// This method ensures the post condition of `self.avail() <= max_avail`.
    pub const fn limit(mut self, max_avail: usize) -> Self {
        if max_avail < self.avail() {
            // SAFETY: the new end is less than the old end.
            unsafe { self.end = self.cursor.add(max_avail) };
        }
        self
    }

    /// Skips the first `nbytes` bytes of data.
    /// The length of available space is decreased accordingly.
    ///
    /// # Panic
    ///
    /// If `nbytes` is greater than `self.avail()`, then the method panics.
    pub fn skip(mut self, nbytes: usize) -> Self {
        assert!(nbytes <= self.avail());

        // SAFETY: the new cursor is less than or equal to the end.
        unsafe { self.cursor = self.cursor.add(nbytes) };
        self
    }
}

impl<'a> From<&'a mut [u8]> for VmWriter<'a> {
    fn from(slice: &'a mut [u8]) -> Self {
        // SAFETY: the range of memory is contiguous and is valid during `'a`, and
        // will not overlap with other `VmWriter`s and `VmReader`s since the slice
        // already has an mutable reference. The slice will not be mapped to the user
        // space hence it will also not overlap with `VmWriter`s and `VmReader`s
        // generated from user space.
        unsafe { Self::from_kernel_space(slice.as_mut_ptr(), slice.len()) }
    }
}
