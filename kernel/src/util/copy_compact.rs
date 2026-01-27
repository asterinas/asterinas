// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

/// A trait that can copies structures from/to the user space in a backward-compatible way.
pub trait CopyCompat {
    /// Reads a POD structure from the user space in a backward-compatible way.
    ///
    /// The method's behavior is described below to ensure backward compatibility:
    ///  - If the `size` specified by the user is greater than the kernel structure's size (i.e.,
    ///    `size_of::<T>()`), the kernel structure's trailing part will be filled with zero.
    ///  - If the `size` specified by the user is smaller than the kernel structure's size (i.e.,
    ///    `size_of::<T>()`), this method will fail with [`Errno::E2BIG`] if the user structure's
    ///    trailing part contains non-zero values.
    fn read_val_compat<T: Pod>(&self, src: Vaddr, size: usize) -> Result<T>;

    /// Writes a POD structure to the user space in a backward-compatible way.
    ///
    /// The method's behavior is described below to ensure backward compatibility:
    ///  - If the `size` specified by the user is greater than the kernel structure's size (i.e.,
    ///    `size_of::<T>()`), the user structure's trailing part will be filled with zero.
    ///  - If the `size` specified by the user is smaller than the kernel structure's size (i.e.,
    ///    `size_of::<T>()`), this method will return a [`TrailingBytes`] instance and the caller
    ///    can use it to check if the kernel structure's trailing part contains non-zero values.
    fn write_val_compat<'a, T: Pod>(
        &self,
        dest: Vaddr,
        size: usize,
        val: &'a T,
    ) -> Result<TrailingBytes<'a>>;
}

impl CopyCompat for CurrentUserSpace<'_> {
    fn read_val_compat<T: Pod>(&self, src: Vaddr, size: usize) -> Result<T> {
        let mut val = T::new_zeroed();

        let mut reader = self.reader(src, size)?;
        reader.read_fallible(&mut VmWriter::from(val.as_mut_bytes()))?;

        while reader.remain() > size_of::<u64>() {
            if reader.read_val::<u64>()? != 0 {
                return_errno_with_message!(Errno::E2BIG, "the user structure is not compatible");
            }
        }
        while reader.has_remain() {
            if reader.read_val::<u8>()? != 0 {
                return_errno_with_message!(Errno::E2BIG, "the user structure is not compatible");
            }
        }

        Ok(val)
    }

    fn write_val_compat<'a, T: Pod>(
        &self,
        dest: Vaddr,
        size: usize,
        val: &'a T,
    ) -> Result<TrailingBytes<'a>> {
        let mut writer = self.writer(dest, size)?;
        writer.write_fallible(&mut VmReader::from(val.as_bytes()))?;

        if size < size_of::<T>() {
            Ok(TrailingBytes(&val.as_bytes()[size..]))
        } else {
            writer.fill_zeros(writer.avail())?;
            Ok(TrailingBytes(&[]))
        }
    }
}

/// The kernel structure's trailing bytes after [`CopyCompat::write_val_compat`].
#[must_use]
#[expect(dead_code)]
pub struct TrailingBytes<'a>(&'a [u8]);

impl TrailingBytes<'_> {
    /// Ignores the trailing bytes.
    pub fn ignore_trailing(self) {}

    // TODO: Add a `check_trailing` method to check if the trailing bytes contain non-zero values.
}
