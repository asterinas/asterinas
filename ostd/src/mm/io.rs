// SPDX-License-Identifier: MPL-2.0

//! Abstractions for reading and writing virtual memory (VM) objects.
//!
//! # Safety
//!
//! The core virtual memory (VM) access APIs provided by this module are [`VmReader`] and
//! [`VmWriter`], which allow for writing to or reading from a region of memory _safely_.
//! `VmReader` and `VmWriter` objects can be constructed from memory regions of either typed memory
//! (e.g., `&[u8]`) or untyped memory (e.g, [`UFrame`]). Behind the scene, `VmReader` and `VmWriter`
//! must be constructed via their [`from_user_space`] and [`from_kernel_space`] methods, whose
//! safety depends on whether the given memory regions are _valid_ or not.
//!
//! [`UFrame`]: crate::mm::UFrame
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

use core::{marker::PhantomData, mem::MaybeUninit};

use crate::{
    arch::mm::{
        __atomic_cmpxchg_fallible, __atomic_load_fallible, __memcpy_fallible, __memset_fallible,
    },
    mm::{
        kspace::{KERNEL_BASE_VADDR, KERNEL_END_VADDR},
        MAX_USERSPACE_VADDR,
    },
    prelude::*,
    Error, Pod,
};

/// A trait that enables reading/writing data from/to a VM object,
/// e.g., [`USegment`], [`Vec<UFrame>`] and [`UFrame`].
///
/// # Concurrency
///
/// The methods may be executed by multiple concurrent reader and writer
/// threads. In this case, if the results of concurrent reads or writes
/// desire predictability or atomicity, the users should add extra mechanism
/// for such properties.
///
/// [`USegment`]: crate::mm::USegment
/// [`UFrame`]: crate::mm::UFrame
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
        // Why not use `MaybeUninit` for a faster implementation?
        //
        // ```rust
        // let mut val: MaybeUninit<T> = MaybeUninit::uninit();
        // let writer = unsafe {
        //     VmWriter::from_kernel_space(val.as_mut_ptr().cast(), core::mem::size_of::<T>())
        // };
        // self.read(offset, &mut writer.to_fallible())?;
        // Ok(unsafe { val.assume_init() })
        // ```
        //
        // The above implementation avoids initializing `val` upfront,
        // so it is more efficient than our actual implementation.
        // Unfortunately, it is unsound.
        // This is because the `read` method,
        // which could be implemented outside OSTD and thus is untrusted,
        // may not really initialize the bits of `val` at all!

        let mut val = T::new_zeroed();
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
}

/// A trait that enables filling bytes (e.g., filling zeros) to a VM object.
pub trait VmIoFill {
    /// Writes `len` zeros at a specified offset.
    ///
    /// Unlike the methods in [`VmIo`], this method allows for short writes because `len` can be
    /// effectively unbounded. However, if not all bytes can be written successfully, an `Err(_)`
    /// will be returned with the error and the number of zeros that have been written thus far.
    ///
    /// # A slow, general implementation
    ///
    /// Suppose that [`VmIo`] has already been implemented for the type,
    /// this method can be implemented in the following general way.
    ///
    /// ```rust
    /// fn fill_zeros(&self, offset: usize, len: usize) -> core::result::Result<(), (Error, usize)> {
    ///     for i in 0..len {
    ///         match self.write_slice(offset + i, &[0u8]) {
    ///             Ok(()) => continue,
    ///             Err(err) => return Err((err, i)),
    ///         }
    ///     }
    ///     Ok(())
    /// }
    /// ```
    ///
    /// But we choose not to provide a general, default implementation
    /// because doing so would make it too easy for a concrete type of `VmIoFill`
    /// to settle with a slower implementation for such a performance-sensitive operation.
    fn fill_zeros(&self, offset: usize, len: usize) -> core::result::Result<(), (Error, usize)>;
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

/// A marker type used for [`VmReader`] and [`VmWriter`],
/// representing whether reads or writes on the underlying memory region are fallible.
pub enum Fallible {}

/// A marker type used for [`VmReader`] and [`VmWriter`],
/// representing whether reads or writes on the underlying memory region are infallible.
pub enum Infallible {}

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

    // SAFETY: The safety is guaranteed by the safety preconditions and the explanation above.
    unsafe { core::intrinsics::volatile_copy_memory(dst, src, len) };
}

/// Fills `len` bytes of memory at `dst` with the specified `value`.
///
/// # Safety
///
/// - `dst` must be [valid] for writes of `len` bytes.
///
/// [valid]: crate::mm::io#safety
unsafe fn memset(dst: *mut u8, value: u8, len: usize) {
    // SAFETY: The safety is guaranteed by the safety preconditions and the explanation above.
    unsafe {
        core::intrinsics::volatile_set_memory(dst, value, len);
    }
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
    // SAFETY: The safety is upheld by the caller.
    let failed_bytes = unsafe { __memcpy_fallible(dst, src, len) };
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
    // SAFETY: The safety is upheld by the caller.
    let failed_bytes = unsafe { __memset_fallible(dst, value, len) };
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

// `Clone` can be implemented for `VmReader`
// because it either points to untyped memory or represents immutable references.
// Note that we cannot implement `Clone` for `VmWriter`
// because it can represent mutable references, which must remain exclusive.
impl<Fallibility> Clone for VmReader<'_, Fallibility> {
    fn clone(&self) -> Self {
        Self {
            cursor: self.cursor,
            end: self.end,
            phantom: PhantomData,
        }
    }
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
                let copied_len = unsafe { memcpy_fallible(writer.cursor, self.cursor, copy_len) };
                self.cursor = self.cursor.wrapping_add(copied_len);
                writer.cursor = writer.cursor.wrapping_add(copied_len);

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
        debug_assert!(len == 0 || KERNEL_BASE_VADDR <= ptr.addr());
        debug_assert!(len == 0 || ptr.addr().checked_add(len).unwrap() <= KERNEL_END_VADDR);

        Self {
            cursor: ptr,
            end: ptr.wrapping_add(len),
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
        unsafe { memcpy(writer.cursor, self.cursor, copy_len) };
        self.cursor = self.cursor.wrapping_add(copy_len);
        writer.cursor = writer.cursor.wrapping_add(copy_len);

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

        let mut val = MaybeUninit::<T>::uninit();

        // SAFETY:
        // - The memory range points to typed memory.
        // - The validity requirements for write accesses are met because the pointer is converted
        //   from a mutable pointer where the underlying storage outlives the temporary lifetime
        //   and no other Rust references to the same storage exist during the lifetime.
        // - The type, i.e., `T`, is plain-old-data.
        let mut writer = unsafe {
            VmWriter::from_kernel_space(val.as_mut_ptr().cast(), core::mem::size_of::<T>())
        };
        self.read(&mut writer);
        debug_assert!(!writer.has_avail());

        // SAFETY:
        // - `self.read` has initialized all the bytes in `val`.
        // - The type is plain-old-data.
        let val_inited = unsafe { val.assume_init() };
        Ok(val_inited)
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

        const { assert!(pod_once_impls::is_non_tearing::<T>()) };

        // SAFETY: We have checked that the number of bytes remaining is at least the size of `T`
        // and that the cursor is properly aligned with respect to the type `T`. All other safety
        // requirements are the same as for `Self::read`.
        let val = unsafe { cursor.read_volatile() };
        self.cursor = self.cursor.wrapping_add(core::mem::size_of::<T>());

        Ok(val)
    }

    // Currently, there are no volatile atomic operations in `core::intrinsics`. Therefore, we do
    // not provide an infallible implementation of `VmReader::atomic_load`.

    /// Converts to a fallible reader.
    pub fn to_fallible(self) -> VmReader<'a, Fallible> {
        // It is safe to construct a fallible reader since an infallible reader covers the
        // capabilities of a fallible reader.
        VmReader {
            cursor: self.cursor,
            end: self.end,
            phantom: PhantomData,
        }
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
        debug_assert!(ptr.addr().checked_add(len).unwrap() <= MAX_USERSPACE_VADDR);

        Self {
            cursor: ptr,
            end: ptr.wrapping_add(len),
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

        let mut val = MaybeUninit::<T>::uninit();

        // SAFETY:
        // - The memory range points to typed memory.
        // - The validity requirements for write accesses are met because the pointer is converted
        //   from a mutable pointer where the underlying storage outlives the temporary lifetime
        //   and no other Rust references to the same storage exist during the lifetime.
        // - The type, i.e., `T`, is plain-old-data.
        let mut writer = unsafe {
            VmWriter::from_kernel_space(val.as_mut_ptr().cast(), core::mem::size_of::<T>())
        };
        self.read_fallible(&mut writer)
            .map_err(|(err, copied_len)| {
                // The `copied_len` is the number of bytes read so far.
                // So the `cursor` can be moved back to the original position.
                self.cursor = self.cursor.wrapping_sub(copied_len);
                err
            })?;
        debug_assert!(!writer.has_avail());

        // SAFETY:
        // - `self.read_fallible` has initialized all the bytes in `val`.
        // - The type is plain-old-data.
        let val_inited = unsafe { val.assume_init() };
        Ok(val_inited)
    }

    /// Atomically loads a `PodAtomic` value.
    ///
    /// Regardless of whether it is successful, the cursor of the reader will not move.
    ///
    /// This method only guarantees the atomicity of the specific operation. There are no
    /// synchronization constraints on other memory accesses. This aligns with the [Relaxed
    /// ordering](https://en.cppreference.com/w/cpp/atomic/memory_order.html#Relaxed_ordering)
    /// specified in the C++11 memory model.
    ///
    /// This method will fail with errors if
    ///  1. the remaining space of the reader is less than `core::mem::size_of::<T>()` bytes, or
    ///  2. the memory operation fails due to an unresolvable page fault.
    ///
    /// # Panics
    ///
    /// This method will panic if the memory location is not aligned on a
    /// `core::mem::align_of::<T>()`-byte boundary.
    pub fn atomic_load<T: PodAtomic>(&self) -> Result<T> {
        if self.remain() < core::mem::size_of::<T>() {
            return Err(Error::InvalidArgs);
        }

        let cursor = self.cursor.cast::<T>();
        assert!(cursor.is_aligned());

        // SAFETY:
        // 1. The cursor is either valid for reading or in user space for `size_of::<T>()` bytes.
        // 2. The cursor is aligned on a `align_of::<T>()`-byte boundary.
        unsafe { T::atomic_load_fallible(cursor) }
    }
}

impl<Fallibility> VmReader<'_, Fallibility> {
    /// Returns the number of bytes for the remaining data.
    pub fn remain(&self) -> usize {
        self.end.addr() - self.cursor.addr()
    }

    /// Returns the cursor pointer, which refers to the address of the next byte to read.
    pub fn cursor(&self) -> *const u8 {
        self.cursor
    }

    /// Returns if it has remaining data to read.
    pub fn has_remain(&self) -> bool {
        self.remain() > 0
    }

    /// Limits the length of remaining data.
    ///
    /// This method ensures the post condition of `self.remain() <= max_remain`.
    pub fn limit(&mut self, max_remain: usize) -> &mut Self {
        if max_remain < self.remain() {
            self.end = self.cursor.wrapping_add(max_remain);
        }

        self
    }

    /// Skips the first `nbytes` bytes of data.
    /// The length of remaining data is decreased accordingly.
    ///
    /// # Panics
    ///
    /// If `nbytes` is greater than `self.remain()`, then the method panics.
    pub fn skip(&mut self, nbytes: usize) -> &mut Self {
        assert!(nbytes <= self.remain());
        self.cursor = self.cursor.wrapping_add(nbytes);

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
        debug_assert!(len == 0 || KERNEL_BASE_VADDR <= ptr.addr());
        debug_assert!(len == 0 || ptr.addr().checked_add(len).unwrap() <= KERNEL_END_VADDR);

        Self {
            cursor: ptr,
            end: ptr.wrapping_add(len),
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

        const { assert!(pod_once_impls::is_non_tearing::<T>()) };

        // SAFETY: We have checked that the number of bytes remaining is at least the size of `T`
        // and that the cursor is properly aligned with respect to the type `T`. All other safety
        // requirements are the same as for `Self::write`.
        unsafe { cursor.write_volatile(*new_val) };
        self.cursor = self.cursor.wrapping_add(core::mem::size_of::<T>());

        Ok(())
    }

    // Currently, there are no volatile atomic operations in `core::intrinsics`. Therefore, we do
    // not provide an infallible implementation of `VmWriter::atomic_update`.

    /// Writes `len` zeros to the target memory.
    ///
    /// This method attempts to fill up to `len` bytes with zeros. If the available
    /// memory from the current cursor position is less than `len`, it will only fill
    /// the available space.
    pub fn fill_zeros(&mut self, len: usize) -> usize {
        let len_to_set = self.avail().min(len);
        if len_to_set == 0 {
            return 0;
        }

        // SAFETY: The destination is a subset of the memory range specified by
        // the current writer, so it is valid for writing.
        unsafe { memset(self.cursor, 0u8, len_to_set) };
        self.cursor = self.cursor.wrapping_add(len_to_set);

        len_to_set
    }

    /// Converts to a fallible writer.
    pub fn to_fallible(self) -> VmWriter<'a, Fallible> {
        // It is safe to construct a fallible reader since an infallible reader covers the
        // capabilities of a fallible reader.
        VmWriter {
            cursor: self.cursor,
            end: self.end,
            phantom: PhantomData,
        }
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
        debug_assert!(ptr.addr().checked_add(len).unwrap() <= MAX_USERSPACE_VADDR);

        Self {
            cursor: ptr,
            end: ptr.wrapping_add(len),
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
                // The `copied_len` is the number of bytes written so far.
                // So the `cursor` can be moved back to the original position.
                self.cursor = self.cursor.wrapping_sub(copied_len);
                err
            })?;
        Ok(())
    }

    /// Atomically updates a `PodAtomic` value.
    ///
    /// This is implemented by performing an atomic load, applying the operation, and performing an
    /// atomic compare-and-exchange. So this cannot prevent the [ABA
    /// problem](https://en.wikipedia.org/wiki/ABA_problem).
    ///
    /// The caller is required to provide a reader which points to the exactly same memory location
    /// to ensure that reading from the memory is allowed.
    ///
    /// On success, the previous value will be returned with a boolean value denoting whether the
    /// compare-and-exchange succeeds. The caller usually wants to retry if the flag is false.
    ///
    /// Regardless of whether it is successful, the cursor of the reader and writer will not move.
    ///
    /// This method only guarantees the atomicity of the specific operation. There are no
    /// synchronization constraints on other memory accesses. This aligns with the [Relaxed
    /// ordering](https://en.cppreference.com/w/cpp/atomic/memory_order.html#Relaxed_ordering)
    /// specified in the C++11 memory model.
    ///
    /// This method will fail with errors if:
    ///  1. the remaining (avail) space of the reader (writer) is less than
    ///     `core::mem::size_of::<T>()` bytes, or
    ///  2. the memory operation fails due to an unresolvable page fault.
    ///
    /// # Panics
    ///
    /// This method will panic if:
    ///  1. the reader and the writer does not point to the same memory location, or
    ///  2. the memory location is not aligned on a `core::mem::align_of::<T>()`-byte boundary.
    pub fn atomic_update<T>(
        &mut self,
        reader: &VmReader,
        op: impl FnOnce(T) -> T,
    ) -> Result<(T, bool)>
    where
        T: PodAtomic + Eq,
    {
        if self.avail() < core::mem::size_of::<T>() || reader.remain() < core::mem::size_of::<T>() {
            return Err(Error::InvalidArgs);
        }

        assert_eq!(self.cursor.cast_const(), reader.cursor);

        let cursor = self.cursor.cast::<T>();
        assert!(cursor.is_aligned());

        // SAFETY:
        // 1. The cursor is either valid for reading or in user space for `size_of::<T>()` bytes.
        // 2. The cursor is aligned on a `align_of::<T>()`-byte boundary.
        let old_val = unsafe { T::atomic_load_fallible(cursor)? };

        let new_val = op(old_val);

        // SAFETY:
        // 1. The cursor is either valid for reading and writing or in user space for 4 bytes.
        // 2. The cursor is aligned on a 4-byte boundary.
        let cur_val = unsafe { T::atomic_cmpxchg_fallible(cursor, old_val, new_val)? };

        Ok((old_val, old_val == cur_val))
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
        let set_len = unsafe { memset_fallible(self.cursor, 0u8, len_to_set) };
        self.cursor = self.cursor.wrapping_add(set_len);

        if set_len < len_to_set {
            Err((Error::PageFault, set_len))
        } else {
            Ok(len_to_set)
        }
    }
}

impl<Fallibility> VmWriter<'_, Fallibility> {
    /// Returns the number of bytes for the available space.
    pub fn avail(&self) -> usize {
        self.end.addr() - self.cursor.addr()
    }

    /// Returns the cursor pointer, which refers to the address of the next byte to write.
    pub fn cursor(&self) -> *mut u8 {
        self.cursor
    }

    /// Returns if it has available space to write.
    pub fn has_avail(&self) -> bool {
        self.avail() > 0
    }

    /// Limits the length of available space.
    ///
    /// This method ensures the post condition of `self.avail() <= max_avail`.
    pub fn limit(&mut self, max_avail: usize) -> &mut Self {
        if max_avail < self.avail() {
            self.end = self.cursor.wrapping_add(max_avail);
        }

        self
    }

    /// Skips the first `nbytes` bytes of data.
    /// The length of available space is decreased accordingly.
    ///
    /// # Panics
    ///
    /// If `nbytes` is greater than `self.avail()`, then the method panics.
    pub fn skip(&mut self, nbytes: usize) -> &mut Self {
        assert!(nbytes <= self.avail());
        self.cursor = self.cursor.wrapping_add(nbytes);

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
/// This trait is mostly a hint, since it's safe and can be implemented for _any_ POD type. If it
/// is implemented for a type that cannot be read or written with a single instruction, calling
/// `read_once`/`write_once` will lead to a failed compile-time assertion.
pub trait PodOnce: Pod {}

#[cfg(any(
    target_arch = "x86_64",
    target_arch = "riscv64",
    target_arch = "loongarch64"
))]
mod pod_once_impls {
    use super::PodOnce;

    impl PodOnce for u8 {}
    impl PodOnce for u16 {}
    impl PodOnce for u32 {}
    impl PodOnce for u64 {}
    impl PodOnce for usize {}
    impl PodOnce for i8 {}
    impl PodOnce for i16 {}
    impl PodOnce for i32 {}
    impl PodOnce for i64 {}
    impl PodOnce for isize {}

    /// Checks whether the memory operation created by `ptr::read_volatile` and
    /// `ptr::write_volatile` doesn't tear.
    ///
    /// Note that the Rust documentation makes no such guarantee, and even the wording in the LLVM
    /// LangRef is ambiguous. But this is unlikely to break in practice because the Linux kernel
    /// also uses "volatile" semantics to implement `READ_ONCE`/`WRITE_ONCE`.
    pub(super) const fn is_non_tearing<T>() -> bool {
        let size = core::mem::size_of::<T>();

        size == 1 || size == 2 || size == 4 || size == 8
    }
}

/// A marker trait for POD types that can be read or written atomically.
pub trait PodAtomic: Pod {
    /// Atomically loads a value.
    /// This function will return errors if encountering an unresolvable page fault.
    ///
    /// Returns the loaded value.
    ///
    /// # Safety
    ///
    /// - `ptr` must either be [valid] for writes of `core::mem::size_of::<T>()` bytes or be in user
    ///   space for  `core::mem::size_of::<T>()` bytes.
    /// - `ptr` must be aligned on a `core::mem::align_of::<T>()`-byte boundary.
    ///
    /// [valid]: crate::mm::io#safety
    #[doc(hidden)]
    unsafe fn atomic_load_fallible(ptr: *const Self) -> Result<Self>;

    /// Atomically compares and exchanges a value.
    /// This function will return errors if encountering an unresolvable page fault.
    ///
    /// Returns the previous value.
    /// `new_val` will be written if and only if the previous value is equal to `old_val`.
    ///
    /// # Safety
    ///
    /// - `ptr` must either be [valid] for writes of `core::mem::size_of::<T>()` bytes or be in user
    ///   space for  `core::mem::size_of::<T>()` bytes.
    /// - `ptr` must be aligned on a `core::mem::align_of::<T>()`-byte boundary.
    ///
    /// [valid]: crate::mm::io#safety
    #[doc(hidden)]
    unsafe fn atomic_cmpxchg_fallible(ptr: *mut Self, old_val: Self, new_val: Self)
        -> Result<Self>;
}

impl PodAtomic for u32 {
    unsafe fn atomic_load_fallible(ptr: *const Self) -> Result<Self> {
        // SAFETY: The safety is upheld by the caller.
        let result = unsafe { __atomic_load_fallible(ptr) };
        if result == !0 {
            Err(Error::PageFault)
        } else {
            Ok(result as Self)
        }
    }

    unsafe fn atomic_cmpxchg_fallible(ptr: *mut Self, old_val: Self, new_val: Self) -> Result<u32> {
        // SAFETY: The safety is upheld by the caller.
        let result = unsafe { __atomic_cmpxchg_fallible(ptr, old_val, new_val) };
        if result == !0 {
            Err(Error::PageFault)
        } else {
            Ok(result as Self)
        }
    }
}
