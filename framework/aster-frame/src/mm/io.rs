// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use core::marker::PhantomData;

use align_ext::AlignExt;
use inherit_methods_macro::inherit_methods;
use pod::Pod;

use crate::prelude::*;

/// A trait that enables reading/writing data from/to a VM object,
/// e.g., [`VmSpace`], [`FrameVec`], and [`Frame`].
///
/// # Concurrency
///
/// The methods may be executed by multiple concurrent reader and writer
/// threads. In this case, if the results of concurrent reads or writes
/// desire predictability or atomicity, the users should add extra mechanism
/// for such properties.
///
/// [`VmSpace`]: crate::mm::VmSpace
/// [`FrameVec`]: crate::mm::FrameVec
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

/// VmReader is a reader for reading data from a contiguous range of memory.
pub struct VmReader<'a> {
    cursor: *const u8,
    end: *const u8,
    phantom: PhantomData<&'a [u8]>,
}

impl<'a> VmReader<'a> {
    /// Constructs a VmReader from a pointer and a length.
    ///
    /// # Safety
    ///
    /// User must ensure the memory from `ptr` to `ptr.add(len)` is contiguous.
    /// User must ensure the memory is valid during the entire period of `'a`.
    pub const unsafe fn from_raw_parts(ptr: *const u8, len: usize) -> Self {
        Self {
            cursor: ptr,
            end: ptr.add(len),
            phantom: PhantomData,
        }
    }

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
    /// This method ensures the postcondition of `self.remain() <= max_remain`.
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

    /// Reads all data into the writer until one of the two conditions is met:
    /// 1. The reader has no remaining data.
    /// 2. The writer has no available space.
    ///
    /// Returns the number of bytes read.
    ///
    /// It pulls the number of bytes data from the reader and
    /// fills in the writer with the number of bytes.
    pub fn read(&mut self, writer: &mut VmWriter<'_>) -> usize {
        let copy_len = self.remain().min(writer.avail());
        if copy_len == 0 {
            return 0;
        }

        // SAFETY: the memory range is valid since `copy_len` is the minimum
        // of the reader's remaining data and the writer's available space.
        unsafe {
            core::ptr::copy(self.cursor, writer.cursor, copy_len);
            self.cursor = self.cursor.add(copy_len);
            writer.cursor = writer.cursor.add(copy_len);
        }
        copy_len
    }

    /// Read a value of `Pod` type.
    ///
    /// # Panic
    ///
    /// If the length of the `Pod` type exceeds `self.remain()`, then this method will panic.
    pub fn read_val<T: Pod>(&mut self) -> T {
        assert!(self.remain() >= core::mem::size_of::<T>());

        let mut val = T::new_uninit();
        let mut writer = VmWriter::from(val.as_bytes_mut());
        let read_len = self.read(&mut writer);

        val
    }
}

impl<'a> From<&'a [u8]> for VmReader<'a> {
    fn from(slice: &'a [u8]) -> Self {
        // SAFETY: the range of memory is contiguous and is valid during `'a`.
        unsafe { Self::from_raw_parts(slice.as_ptr(), slice.len()) }
    }
}

/// VmWriter is a writer for writing data to a contiguous range of memory.
pub struct VmWriter<'a> {
    cursor: *mut u8,
    end: *mut u8,
    phantom: PhantomData<&'a mut [u8]>,
}

impl<'a> VmWriter<'a> {
    /// Constructs a VmWriter from a pointer and a length.
    ///
    /// # Safety
    ///
    /// User must ensure the memory from `ptr` to `ptr.add(len)` is contiguous.
    /// User must ensure the memory is valid during the entire period of `'a`.
    pub const unsafe fn from_raw_parts_mut(ptr: *mut u8, len: usize) -> Self {
        Self {
            cursor: ptr,
            end: ptr.add(len),
            phantom: PhantomData,
        }
    }

    /// Returns the number of bytes for the available space.
    pub const fn avail(&self) -> usize {
        // SAFETY: the end is equal to or greater than the cursor.
        unsafe { self.end.sub_ptr(self.cursor) }
    }

    /// Returns the cursor pointer, which refers to the address of the next byte to write.
    pub const fn cursor(&self) -> *mut u8 {
        self.cursor
    }

    /// Returns if it has avaliable space to write.
    pub const fn has_avail(&self) -> bool {
        self.avail() > 0
    }

    /// Limits the length of available space.
    ///
    /// This method ensures the postcondition of `self.avail() <= max_avail`.
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

    /// Writes data from the reader until one of the two conditions is met:
    /// 1. The writer has no available space.
    /// 2. The reader has no remaining data.
    ///
    /// Returns the number of bytes written.
    ///
    /// It pulls the number of bytes data from the reader and
    /// fills in the writer with the number of bytes.
    pub fn write(&mut self, reader: &mut VmReader<'_>) -> usize {
        let copy_len = self.avail().min(reader.remain());
        if copy_len == 0 {
            return 0;
        }

        // SAFETY: the memory range is valid since `copy_len` is the minimum
        // of the reader's remaining data and the writer's available space.
        unsafe {
            core::ptr::copy(reader.cursor, self.cursor, copy_len);
            self.cursor = self.cursor.add(copy_len);
            reader.cursor = reader.cursor.add(copy_len);
        }
        copy_len
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

impl<'a> From<&'a mut [u8]> for VmWriter<'a> {
    fn from(slice: &'a mut [u8]) -> Self {
        // SAFETY: the range of memory is contiguous and is valid during `'a`.
        unsafe { Self::from_raw_parts_mut(slice.as_mut_ptr(), slice.len()) }
    }
}
