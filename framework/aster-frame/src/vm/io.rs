// SPDX-License-Identifier: MPL-2.0

use align_ext::AlignExt;
use inherit_methods_macro::inherit_methods;
use pod::Pod;

use crate::prelude::*;

/// A trait that enables reading/writing data from/to a VM object,
/// e.g., `VmSpace`, `VmFrameVec`, and `VmFrame`.
///
/// # Concurrency
///
/// The methods may be executed by multiple concurrent reader and writer
/// threads. In this case, if the results of concurrent reads or writes
/// desire predictability or atomicity, the users should add extra mechanism
/// for such properties.
pub trait VmIo: Send + Sync {
    /// Read a specified number of bytes at a specified offset into a given buffer.
    ///
    /// # No short reads
    ///
    /// On success, the output `buf` must be filled with the requested data
    /// completely. If, for any reason, the requested data is only partially
    /// available, then the method shall return an error.
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()>;

    /// Read a value of a specified type at a specified offset.
    fn read_val<T: Pod>(&self, offset: usize) -> Result<T> {
        let mut val = T::new_uninit();
        self.read_bytes(offset, val.as_bytes_mut())?;
        Ok(val)
    }

    /// Read a slice of a specified type at a specified offset.
    ///
    /// # No short reads
    ///
    /// Similar to `read_bytes`.
    fn read_slice<T: Pod>(&self, offset: usize, slice: &mut [T]) -> Result<()> {
        let buf = unsafe { core::mem::transmute(slice) };
        self.read_bytes(offset, buf)
    }

    /// Write a specified number of bytes from a given buffer at a specified offset.
    ///
    /// # No short writes
    ///
    /// On success, the input `buf` must be written to the VM object entirely.
    /// If, for any reason, the input data can only be written partially,
    /// then the method shall return an error.
    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()>;

    /// Write a value of a specified type at a specified offset.
    fn write_val<T: Pod>(&self, offset: usize, new_val: &T) -> Result<()> {
        self.write_bytes(offset, new_val.as_bytes())?;
        Ok(())
    }

    /// Write a slice of a specified type at a specified offset.
    ///
    /// # No short write
    ///
    /// Similar to `write_bytes`.
    fn write_slice<T: Pod>(&self, offset: usize, slice: &[T]) -> Result<()> {
        let buf = unsafe { core::mem::transmute(slice) };
        self.write_bytes(offset, buf)
    }

    /// Write a sequence of values given by an iterator (`iter`) from the specified offset (`offset`).
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
    /// Initializing an VM object with the same value can be done easily with `write_values`.
    ///
    /// ```
    /// use core::iter::self;
    ///
    /// let _nr_values = vm_obj.write_values(0, iter::repeat(0_u32), 0).unwrap();
    /// ```
    ///
    /// # Panic
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
