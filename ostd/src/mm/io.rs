// SPDX-License-Identifier: MPL-2.0

//! Abstractions for reading and writing virtual memory (VM) objects.
//!
//! # Safety
//!
//! The core virtual memory (VM) access APIs provided by this module are [`VmReader`] and
//! [`VmWriter`], which allow for writing to or reading from a region of memory _safely_.
//! `VmReader` and `VmWriter` objects can be constructed from memory regions of either typed memory
//! (e.g., `&[u8]`) or untyped memory (e.g, [`Frame`]). Behind the scene, `VmReader` and `VmWriter`
//! must be constructed via their [`from_user_space`] and [`from_kernel_space`] methods, whose
//! safety depends on whether the given memory regions are _valid_ or not.
//!
//! [`Frame`]: crate::mm::Frame
//! [`from_user_space`]: `VmReader::from_user_space`
//! [`from_kernel_space`]: `VmReader::from_kernel_space`
//!
//! Here is a list of conditions for memory regions to be considered valid:
//!
//! - The memory region as a whole must be either typed or untyped memory, not both typed and
//!   untyped.
//!
//! - If the memory region is typed, we require that:
//!   - the [validity requirements] from the official Rust documentation must be met, and
//!   - the type of the memory region (which must exist since the memory is typed) must be
//!     plain-old-data, so that the writer can fill it with arbitrary data safely.
//!
//! [validity requirements]: core::ptr#safety
//!
//! - If the memory region is untyped, we require that:
//!   - the underlying pages must remain alive while the validity requirements are in effect, and
//!   - the kernel must access the memory region using only the APIs provided in this module, but
//!     external accesses from hardware devices or user programs do not count.
//!
//! We have the last requirement for untyped memory to be valid because the safety interaction with
//! other ways to access the memory region (e.g., atomic/volatile memory loads/stores) is not
//! currently specified. Tis may be relaxed in the future, if appropriate and necessary.
//!
//! Note that data races on untyped memory are explicitly allowed (since pages can be mapped to
//! user space, making it impossible to avoid data races). However, they may produce erroneous
//! results, such as unexpected bytes being copied, but do not cause soundness problems.

use alloc::vec;
use core::marker::PhantomData;

use align_ext::AlignExt;
use const_assert::{Assert, IsTrue};
use inherit_methods_macro::inherit_methods;

use crate::{
    arch::mm::{__memcpy_fallible, __memset_fallible},
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
    /// Reads requested data at a specified offset into a given `VmWriter`.
    ///
    /// # No short reads
    ///
    /// On success, the `writer` must be written with the requested data
    /// completely. If, for any reason, the requested data is only partially
    /// available, then the method shall return an error.
    fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()>;

    /// Reads a specified number of bytes at a specified offset into a given buffer.
    ///
    /// # No short reads
    ///
    /// Similar to [`read`].
    ///
    /// [`read`]: VmIo::read
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        let mut writer = VmWriter::from(buf).to_fallible();
        self.read(offset, &mut writer)
    }

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
    /// Similar to [`read`].
    ///
    /// [`read`]: VmIo::read
    fn read_slice<T: Pod>(&self, offset: usize, slice: &mut [T]) -> Result<()> {
        let len_in_bytes = core::mem::size_of_val(slice);
        let ptr = slice as *mut [T] as *mut u8;
        // SAFETY: the slice can be transmuted to a writable byte slice since the elements
        // are all Plain-Old-Data (Pod) types.
        let buf = unsafe { core::slice::from_raw_parts_mut(ptr, len_in_bytes) };
        self.read_bytes(offset, buf)
    }

    /// Writes all data from a given `VmReader` at a specified offset.
    ///
    /// # No short writes
    ///
    /// On success, the data from the `reader` must be read to the VM object entirely.
    /// If, for any reason, the input data can only be written partially,
    /// then the method shall return an error.
    fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()>;

    /// Writes a specified number of bytes from a given buffer at a specified offset.
    ///
    /// # No short writes
    ///
    /// Similar to [`write`].
    ///
    /// [`write`]: VmIo::write
    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        let mut reader = VmReader::from(buf).to_fallible();
        self.write(offset, &mut reader)
    }

    /// Writes a value of a specified type at a specified offset.
    fn write_val<T: Pod>(&self, offset: usize, new_val: &T) -> Result<()> {
        self.write_bytes(offset, new_val.as_bytes())?;
        Ok(())
    }

    /// Writes a slice of a specified type at a specified offset.
    ///
    /// # No short write
    ///
    /// Similar to [`write`].
    ///
    /// [`write`]: VmIo::write
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

/// A trait that enables reading/writing data from/to a VM object using one non-tearing memory
/// load/store.
///
/// See also [`VmIo`], which enables reading/writing data from/to a VM object without the guarantee
/// of using one non-tearing memory load/store.
pub trait VmIoOnce {
    /// Reads a value of the `PodOnce` type at the specified offset using one non-tearing memory
    /// load.
    ///
    /// Except that the offset is specified explicitly, the semantics of this method is the same as
    /// [`VmReader::read_once`].
    fn read_once<T: PodOnce>(&self, offset: usize) -> Result<T>;

    /// Writes a value of the `PodOnce` type at the specified offset using one non-tearing memory
    /// store.
    ///
    /// Except that the offset is specified explicitly, the semantics of this method is the same as
    /// [`VmWriter::write_once`].
    fn write_once<T: PodOnce>(&self, offset: usize, new_val: &T) -> Result<()>;
}

macro_rules! impl_vm_io_pointer {
    ($typ:ty,$from:tt) => {
        #[inherit_methods(from = $from)]
        impl<T: VmIo> VmIo for $typ {
            fn read(&self, offset: usize, writer: &mut VmWriter) -> Result<()>;
            fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()>;
            fn read_val<F: Pod>(&self, offset: usize) -> Result<F>;
            fn read_slice<F: Pod>(&self, offset: usize, slice: &mut [F]) -> Result<()>;
            fn write(&self, offset: usize, reader: &mut VmReader) -> Result<()>;
            fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()>;
            fn write_val<F: Pod>(&self, offset: usize, new_val: &F) -> Result<()>;
            fn write_slice<F: Pod>(&self, offset: usize, slice: &[F]) -> Result<()>;
        }
    };
}

impl_vm_io_pointer!(&T, "(**self)");
impl_vm_io_pointer!(&mut T, "(**self)");
impl_vm_io_pointer!(Box<T>, "(**self)");
impl_vm_io_pointer!(Arc<T>, "(**self)");

macro_rules! impl_vm_io_once_pointer {
    ($typ:ty,$from:tt) => {
        #[inherit_methods(from = $from)]
        impl<T: VmIoOnce> VmIoOnce for $typ {
            fn read_once<F: PodOnce>(&self, offset: usize) -> Result<F>;
            fn write_once<F: PodOnce>(&self, offset: usize, new_val: &F) -> Result<()>;
        }
    };
}

impl_vm_io_once_pointer!(&T, "(**self)");
impl_vm_io_once_pointer!(&mut T, "(**self)");
impl_vm_io_once_pointer!(Box<T>, "(**self)");
impl_vm_io_once_pointer!(Arc<T>, "(**self)");

/// A marker structure used for [`VmReader`] and [`VmWriter`],
/// representing whether reads or writes on the underlying memory region are fallible.
pub struct Fallible;
/// A marker structure used for [`VmReader`] and [`VmWriter`],
/// representing whether reads or writes on the underlying memory region are infallible.
pub struct Infallible;

/// Copies `len` bytes from `src` to `dst`.
///
/// # Safety
///
/// - `src` must be [valid] for reads of `len` bytes.
/// - `dst` must be [valid] for writes of `len` bytes.
///
/// [valid]: crate::mm::io#safety
unsafe fn memcpy(dst: *mut u8, src: *const u8, len: usize) {
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
/// In the following cases, this method may cause unexpected bytes to be copied, but will not cause
/// safety problems as long as the safety requirements are met:
/// - The source and destination overlap.
/// - The current context is not associated with valid user space (e.g., in the kernel thread).
///
/// # Safety
///
/// - `src` must either be [valid] for reads of `len` bytes or be in user space for `len` bytes.
/// - `dst` must either be [valid] for writes of `len` bytes or be in user space for `len` bytes.
///
/// [valid]: crate::mm::io#safety
unsafe fn memcpy_fallible(dst: *mut u8, src: *const u8, len: usize) -> usize {
    let failed_bytes = __memcpy_fallible(dst, src, len);
    len - failed_bytes
}

/// Fills `len` bytes of memory at `dst` with the specified `value`.
/// This function will early stop filling if encountering an unresolvable page fault.
///
/// Returns the number of successfully set bytes.
///
/// # Safety
///
/// - `dst` must either be [valid] for writes of `len` bytes or be in user space for `len` bytes.
///
/// [valid]: crate::mm::io#safety
unsafe fn memset_fallible(dst: *mut u8, value: u8, len: usize) -> usize {
    let failed_bytes = __memset_fallible(dst, value, len);
    len - failed_bytes
}

/// Fallible memory read from a `VmWriter`.
pub trait FallibleVmRead<F> {
    /// Reads all data into the writer until one of the three conditions is met:
    /// 1. The reader has no remaining data.
    /// 2. The writer has no available space.
    /// 3. The reader/writer encounters some error.
    ///
    /// On success, the number of bytes read is returned;
    /// On error, both the error and the number of bytes read so far are returned.
    fn read_fallible(
        &mut self,
        writer: &mut VmWriter<'_, F>,
    ) -> core::result::Result<usize, (Error, usize)>;
}

/// Fallible memory write from a `VmReader`.
pub trait FallibleVmWrite<F> {
    /// Writes all data from the reader until one of the three conditions is met:
    /// 1. The reader has no remaining data.
    /// 2. The writer has no available space.
    /// 3. The reader/writer encounters some error.
    ///
    /// On success, the number of bytes written is returned;
    /// On error, both the error and the number of bytes written so far are returned.
    fn write_fallible(
        &mut self,
        reader: &mut VmReader<'_, F>,
    ) -> core::result::Result<usize, (Error, usize)>;
}

/// `VmReader` is a reader for reading data from a contiguous range of memory.
///
/// The memory range read by `VmReader` can be in either kernel space or user space.
/// When the operating range is in kernel space, the memory within that range
/// is guaranteed to be valid, and the corresponding memory reads are infallible.
/// When the operating range is in user space, it is ensured that the page table of
/// the process creating the `VmReader` is active for the duration of `'a`,
/// and the corresponding memory reads are considered fallible.
///
/// When perform reading with a `VmWriter`, if one of them represents typed memory,
/// it can ensure that the reading range in this reader and writing range in the
/// writer are not overlapped.
///
/// NOTE: The overlap mentioned above is at both the virtual address level
/// and physical address level. There is not guarantee for the operation results
/// of `VmReader` and `VmWriter` in overlapping untyped addresses, and it is
/// the user's responsibility to handle this situation.
pub struct VmReader<'a, Fallibility = Fallible> {
    cursor: *const u8,
    end: *const u8,
    phantom: PhantomData<(&'a [u8], Fallibility)>,
}

macro_rules! impl_read_fallible {
    ($reader_fallibility:ty, $writer_fallibility:ty) => {
        impl<'a> FallibleVmRead<$writer_fallibility> for VmReader<'a, $reader_fallibility> {
            fn read_fallible(
                &mut self,
                writer: &mut VmWriter<'_, $writer_fallibility>,
            ) -> core::result::Result<usize, (Error, usize)> {
                let copy_len = self.remain().min(writer.avail());
                if copy_len == 0 {
                    return Ok(0);
                }

                // SAFETY: The source and destination are subsets of memory ranges specified by
                // the reader and writer, so they are either valid for reading and writing or in
                // user space.
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
    ($writer_fallibility:ty, $reader_fallibility:ty) => {
        impl<'a> FallibleVmWrite<$reader_fallibility> for VmWriter<'a, $writer_fallibility> {
            fn write_fallible(
                &mut self,
                reader: &mut VmReader<'_, $reader_fallibility>,
            ) -> core::result::Result<usize, (Error, usize)> {
                reader.read_fallible(self)
            }
        }
    };
}

impl_read_fallible!(Fallible, Infallible);
impl_read_fallible!(Fallible, Fallible);
impl_read_fallible!(Infallible, Fallible);
impl_write_fallible!(Fallible, Infallible);
impl_write_fallible!(Fallible, Fallible);
impl_write_fallible!(Infallible, Fallible);

impl<'a> VmReader<'a, Infallible> {
    /// Constructs a `VmReader` from a pointer and a length, which represents
    /// a memory range in kernel space.
    ///
    /// # Safety
    ///
    /// `ptr` must be [valid] for reads of `len` bytes during the entire lifetime `a`.
    ///
    /// [valid]: crate::mm::io#safety
    pub unsafe fn from_kernel_space(ptr: *const u8, len: usize) -> Self {
        // Rust is allowed to give the reference to a zero-sized object a very small address,
        // falling out of the kernel virtual address space range.
        // So when `len` is zero, we should not and need not to check `ptr`.
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
    pub fn read(&mut self, writer: &mut VmWriter<'_, Infallible>) -> usize {
        let copy_len = self.remain().min(writer.avail());
        if copy_len == 0 {
            return 0;
        }

        // SAFETY: The source and destination are subsets of memory ranges specified by the reader
        // and writer, so they are valid for reading and writing.
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

    /// Reads a value of the `PodOnce` type using one non-tearing memory load.
    ///
    /// If the length of the `PodOnce` type exceeds `self.remain()`, this method will return `Err`.
    ///
    /// This method will not compile if the `Pod` type is too large for the current architecture
    /// and the operation must be tear into multiple memory loads.
    ///
    /// # Panics
    ///
    /// This method will panic if the current position of the reader does not meet the alignment
    /// requirements of type `T`.
    pub fn read_once<T: PodOnce>(&mut self) -> Result<T> {
        if self.remain() < core::mem::size_of::<T>() {
            return Err(Error::InvalidArgs);
        }

        let cursor = self.cursor.cast::<T>();
        assert!(cursor.is_aligned());

        // SAFETY: We have checked that the number of bytes remaining is at least the size of `T`
        // and that the cursor is properly aligned with respect to the type `T`. All other safety
        // requirements are the same as for `Self::read`.
        let val = unsafe { cursor.read_volatile() };
        self.cursor = unsafe { self.cursor.add(core::mem::size_of::<T>()) };

        Ok(val)
    }

    /// Converts to a fallible reader.
    pub fn to_fallible(self) -> VmReader<'a, Fallible> {
        // SAFETY: It is safe to transmute to a fallible reader since
        // 1. the fallibility is a zero-sized marker type,
        // 2. an infallible reader covers the capabilities of a fallible reader.
        unsafe { core::mem::transmute(self) }
    }
}

impl VmReader<'_, Fallible> {
    /// Constructs a `VmReader` from a pointer and a length, which represents
    /// a memory range in user space.
    ///
    /// # Safety
    ///
    /// The virtual address range `ptr..ptr + len` must be in user space.
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
    ///
    /// If the memory read failed, this method will return `Err`
    /// and the current reader's cursor remains pointing to
    /// the original starting position.
    pub fn read_val<T: Pod>(&mut self) -> Result<T> {
        if self.remain() < core::mem::size_of::<T>() {
            return Err(Error::InvalidArgs);
        }

        let mut val = T::new_uninit();
        let mut writer = VmWriter::from(val.as_bytes_mut());
        self.read_fallible(&mut writer)
            .map_err(|(err, copied_len)| {
                // SAFETY: The `copied_len` is the number of bytes read so far.
                // So the `cursor` can be moved back to the original position.
                unsafe {
                    self.cursor = self.cursor.sub(copied_len);
                }
                err
            })?;
        Ok(val)
    }

    /// Collects all the remaining bytes into a `Vec<u8>`.
    ///
    /// If the memory read failed, this method will return `Err`
    /// and the current reader's cursor remains pointing to
    /// the original starting position.
    pub fn collect(&mut self) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; self.remain()];
        self.read_fallible(&mut buf.as_mut_slice().into())
            .map_err(|(err, copied_len)| {
                // SAFETY: The `copied_len` is the number of bytes read so far.
                // So the `cursor` can be moved back to the original position.
                unsafe {
                    self.cursor = self.cursor.sub(copied_len);
                }
                err
            })?;
        Ok(buf)
    }
}

impl<Fallibility> VmReader<'_, Fallibility> {
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
    /// # Panics
    ///
    /// If `nbytes` is greater than `self.remain()`, then the method panics.
    pub fn skip(mut self, nbytes: usize) -> Self {
        assert!(nbytes <= self.remain());

        // SAFETY: the new cursor is less than or equal to the end.
        unsafe { self.cursor = self.cursor.add(nbytes) };
        self
    }
}

impl<'a> From<&'a [u8]> for VmReader<'a, Infallible> {
    fn from(slice: &'a [u8]) -> Self {
        // SAFETY:
        // - The memory range points to typed memory.
        // - The validity requirements for read accesses are met because the pointer is converted
        //   from an immutable reference that outlives the lifetime `'a`.
        // - The type, i.e., the `u8` slice, is plain-old-data.
        unsafe { Self::from_kernel_space(slice.as_ptr(), slice.len()) }
    }
}

/// `VmWriter` is a writer for writing data to a contiguous range of memory.
///
/// The memory range write by `VmWriter` can be in either kernel space or user space.
/// When the operating range is in kernel space, the memory within that range
/// is guaranteed to be valid, and the corresponding memory writes are infallible.
/// When the operating range is in user space, it is ensured that the page table of
/// the process creating the `VmWriter` is active for the duration of `'a`,
/// and the corresponding memory writes are considered fallible.
///
/// When perform writing with a `VmReader`, if one of them represents typed memory,
/// it can ensure that the writing range in this writer and reading range in the
/// reader are not overlapped.
///
/// NOTE: The overlap mentioned above is at both the virtual address level
/// and physical address level. There is not guarantee for the operation results
/// of `VmReader` and `VmWriter` in overlapping untyped addresses, and it is
/// the user's responsibility to handle this situation.
pub struct VmWriter<'a, Fallibility = Fallible> {
    cursor: *mut u8,
    end: *mut u8,
    phantom: PhantomData<(&'a mut [u8], Fallibility)>,
}

impl<'a> VmWriter<'a, Infallible> {
    /// Constructs a `VmWriter` from a pointer and a length, which represents
    /// a memory range in kernel space.
    ///
    /// # Safety
    ///
    /// `ptr` must be [valid] for writes of `len` bytes during the entire lifetime `a`.
    ///
    /// [valid]: crate::mm::io#safety
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
    pub fn write(&mut self, reader: &mut VmReader<'_, Infallible>) -> usize {
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

    /// Writes a value of the `PodOnce` type using one non-tearing memory store.
    ///
    /// If the length of the `PodOnce` type exceeds `self.remain()`, this method will return `Err`.
    ///
    /// # Panics
    ///
    /// This method will panic if the current position of the writer does not meet the alignment
    /// requirements of type `T`.
    pub fn write_once<T: PodOnce>(&mut self, new_val: &T) -> Result<()> {
        if self.avail() < core::mem::size_of::<T>() {
            return Err(Error::InvalidArgs);
        }

        let cursor = self.cursor.cast::<T>();
        assert!(cursor.is_aligned());

        // SAFETY: We have checked that the number of bytes remaining is at least the size of `T`
        // and that the cursor is properly aligned with respect to the type `T`. All other safety
        // requirements are the same as for `Self::writer`.
        unsafe { cursor.cast::<T>().write_volatile(*new_val) };
        self.cursor = unsafe { self.cursor.add(core::mem::size_of::<T>()) };

        Ok(())
    }

    /// Fills the available space by repeating `value`.
    ///
    /// Returns the number of values written.
    ///
    /// # Panics
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
                (self.cursor as *mut T).add(i).write_volatile(value);
            }
        }

        // The available space has been filled so this cursor can be moved to the end.
        self.cursor = self.end;
        written_num
    }

    /// Converts to a fallible writer.
    pub fn to_fallible(self) -> VmWriter<'a, Fallible> {
        // SAFETY: It is safe to transmute to a fallible writer since
        // 1. the fallibility is a zero-sized marker type,
        // 2. an infallible reader covers the capabilities of a fallible reader.
        unsafe { core::mem::transmute(self) }
    }
}

impl VmWriter<'_, Fallible> {
    /// Constructs a `VmWriter` from a pointer and a length, which represents
    /// a memory range in user space.
    ///
    /// The current context should be consistently associated with valid user space during the
    /// entire lifetime `'a`. This is for correct semantics and is not a safety requirement.
    ///
    /// # Safety
    ///
    /// `ptr` must be in user space for `len` bytes.
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
    ///
    /// If the memory write failed, this method will return `Err`
    /// and the current writer's cursor remains pointing to
    /// the original starting position.
    pub fn write_val<T: Pod>(&mut self, new_val: &T) -> Result<()> {
        if self.avail() < core::mem::size_of::<T>() {
            return Err(Error::InvalidArgs);
        }

        let mut reader = VmReader::from(new_val.as_bytes());
        self.write_fallible(&mut reader)
            .map_err(|(err, copied_len)| {
                // SAFETY: The `copied_len` is the number of bytes written so far.
                // So the `cursor` can be moved back to the original position.
                unsafe {
                    self.cursor = self.cursor.sub(copied_len);
                }
                err
            })?;
        Ok(())
    }

    /// Writes `len` zeros to the target memory.
    ///
    /// This method attempts to fill up to `len` bytes with zeros. If the available
    /// memory from the current cursor position is less than `len`, it will only fill
    /// the available space.
    ///
    /// If the memory write failed due to an unresolvable page fault, this method
    /// will return `Err` with the length set so far.
    pub fn fill_zeros(&mut self, len: usize) -> core::result::Result<usize, (Error, usize)> {
        let len_to_set = self.avail().min(len);
        if len_to_set == 0 {
            return Ok(0);
        }

        // SAFETY: The destination is a subset of the memory range specified by
        // the current writer, so it is either valid for writing or in user space.
        let set_len = unsafe {
            let set_len = memset_fallible(self.cursor, 0u8, len_to_set);
            self.cursor = self.cursor.add(set_len);
            set_len
        };
        if set_len < len_to_set {
            Err((Error::PageFault, set_len))
        } else {
            Ok(len_to_set)
        }
    }
}

impl<Fallibility> VmWriter<'_, Fallibility> {
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
    /// # Panics
    ///
    /// If `nbytes` is greater than `self.avail()`, then the method panics.
    pub fn skip(mut self, nbytes: usize) -> Self {
        assert!(nbytes <= self.avail());

        // SAFETY: the new cursor is less than or equal to the end.
        unsafe { self.cursor = self.cursor.add(nbytes) };
        self
    }
}

impl<'a> From<&'a mut [u8]> for VmWriter<'a, Infallible> {
    fn from(slice: &'a mut [u8]) -> Self {
        // SAFETY:
        // - The memory range points to typed memory.
        // - The validity requirements for write accesses are met because the pointer is converted
        //   from a mutable reference that outlives the lifetime `'a`.
        // - The type, i.e., the `u8` slice, is plain-old-data.
        unsafe { Self::from_kernel_space(slice.as_mut_ptr(), slice.len()) }
    }
}

/// A marker trait for POD types that can be read or written with one instruction.
///
/// We currently rely on this trait to ensure that the memory operation created by
/// `ptr::read_volatile` and `ptr::write_volatile` doesn't tear. However, the Rust documentation
/// makes no such guarantee, and even the wording in the LLVM LangRef is ambiguous.
///
/// At this point, we can only _hope_ that this doesn't break in future versions of the Rust or
/// LLVM compilers. However, this is unlikely to happen in practice, since the Linux kernel also
/// uses "volatile" semantics to implement `READ_ONCE`/`WRITE_ONCE`.
pub trait PodOnce: Pod {}

impl<T: Pod> PodOnce for T where Assert<{ is_pod_once::<T>() }>: IsTrue {}

#[cfg(target_arch = "x86_64")]
const fn is_pod_once<T: Pod>() -> bool {
    let size = size_of::<T>();

    size == 1 || size == 2 || size == 4 || size == 8
}

#[cfg(target_arch = "riscv64")]
const fn is_pod_once<T: Pod>() -> bool {
    let size = size_of::<T>();

    size == 1 || size == 2 || size == 4 || size == 8
}
