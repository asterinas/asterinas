// SPDX-License-Identifier: MPL-2.0

use core::cmp::min;

use crate::prelude::*;

/// A trait providing the ability to read a `String` from the user space.
pub trait ReadString {
    /// Reads a `String` from `self` with a maximum length of `max_len`.
    ///   
    /// This method usually needs to allocate a buffer to store the read `String`.
    /// Hence it is crucial to provide a sensible limit to prevent allocating an
    /// excessively large buffer in the kernel.
    fn read_string_with_max_len(&mut self, max_len: usize) -> Result<String>;
}

impl ReadString for VmReader<'_> {
    fn read_string_with_max_len(&mut self, max_len: usize) -> Result<String> {
        let mut buf = vec![0u8; min(self.remain(), max_len)];
        self.read_fallible(&mut VmWriter::from(buf.as_mut_slice()))
            .map_err(|_| Error::new(Errno::EFAULT))?;

        let context = String::from_utf8(buf).map_err(|_| Error::new(Errno::EINVAL))?;

        Ok(context)
    }
}
