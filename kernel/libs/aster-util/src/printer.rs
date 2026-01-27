// SPDX-License-Identifier: MPL-2.0

use core::fmt::{Arguments, Write};

use ostd::mm::{FallibleVmWrite, VmReader, VmWriter};

/// A specialized printer for formatted text output.
///
/// `VmPrinter` is designed to handle the common pattern in kernel where kernel
/// code needs to generate formatted text output (like status information, statistics,
/// or configuration data) and write it to user space with proper offset handling.
///
/// # Examples
///
/// ```rust,ignore
/// use aster_util::printer::VmPrinter;
/// use ostd::{mm::VmWriter, Pod};
///
/// let mut buf = [0u8; 3];
/// let mut writer = VmWriter::from(buf.as_bytes_mut()).to_fallible();
/// let mut printer = VmPrinter::new_skip(&mut writer, 3);
///
/// let res = writeln!(printer, "val: {}", 123);
/// assert!(res.is_ok());
///
/// assert_eq!(printer.bytes_written(), 3);
/// assert_eq!(&buf, b": 1");
/// ```
pub struct VmPrinter<'a, 'b> {
    /// The underlying [`VmWriter`] to write the final output.
    writer: &'a mut VmWriter<'b>,
    /// Number of bytes to skip from the beginning of the output.
    ///
    /// When content is written through this writer, the first `bytes_to_skip`
    /// bytes will be discarded, and only subsequent bytes will be written
    /// to the underlying `VmWriter`.
    bytes_to_skip: usize,
    /// Total number of bytes written to the underlying writer.
    bytes_written: usize,
}

impl<'a, 'b> VmPrinter<'a, 'b> {
    /// Creates a new `VmPrinter` that prints to `writer`.
    fn new(writer: &'a mut VmWriter<'b>) -> Self {
        Self {
            writer,
            bytes_to_skip: 0,
            bytes_written: 0,
        }
    }

    /// Creates a new `VmPrinter` that skips the first `bytes_to_skip` bytes and prints the
    /// remaining bytes to `writer`.
    pub fn new_skip(writer: &'a mut VmWriter<'b>, bytes_to_skip: usize) -> Self {
        Self {
            writer,
            bytes_to_skip,
            bytes_written: 0,
        }
    }

    /// Returns the total number of bytes written to the underlying writer.
    pub fn bytes_written(&self) -> usize {
        self.bytes_written
    }

    /// Writes formatted content to the underlying writer.
    pub fn write_fmt(&mut self, args: Arguments<'_>) -> Result<(), VmPrinterError> {
        Write::write_fmt(self, args).map_err(|_| VmPrinterError::PageFault)
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> core::fmt::Result {
        if self.bytes_to_skip >= bytes.len() {
            self.bytes_to_skip -= bytes.len();
            return Ok(());
        }

        let bytes_to_write = &bytes[self.bytes_to_skip..];
        if self.bytes_to_skip > 0 {
            self.bytes_to_skip = 0;
        }

        let mut reader = VmReader::from(bytes_to_write);
        let written_len = self
            .writer
            .write_fallible(&mut reader)
            .map_err(|_| core::fmt::Error)?;

        self.bytes_written += written_len;

        Ok(())
    }
}

impl Write for VmPrinter<'_, '_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        self.write_bytes(s.as_bytes())
    }
}

impl<'a, 'b> From<&'a mut VmWriter<'b>> for VmPrinter<'a, 'b> {
    fn from(writer: &'a mut VmWriter<'b>) -> Self {
        Self::new(writer)
    }
}

/// An error returned by [`VmPrinter::write_fmt`].
pub enum VmPrinterError {
    /// Page fault occurred.
    PageFault,
}

#[cfg(ktest)]
mod test {
    use ostd::{mm::VmWriter, prelude::*};
    use ostd_pod::IntoBytes;

    use super::*;

    #[ktest]
    fn basic_write() {
        let mut buf = [0u8; 64];
        let mut writer = VmWriter::from(buf.as_mut_bytes()).to_fallible();
        let mut printer = VmPrinter::from(&mut writer);

        let res = writeln!(printer, "test");
        assert!(res.is_ok());

        assert_eq!(printer.bytes_written(), 5);
        assert_eq!(&buf[..5], b"test\n");
    }

    #[ktest]
    fn write_with_skip() {
        let mut buf = [0u8; 3];
        let mut writer = VmWriter::from(buf.as_mut_bytes()).to_fallible();
        let mut printer = VmPrinter::new_skip(&mut writer, 3);

        let res = writeln!(printer, "val: {}", 123);
        assert!(res.is_ok());

        assert_eq!(printer.bytes_written(), 3);
        assert_eq!(&buf, b": 1");
    }

    #[ktest]
    fn skip_all_content() {
        let mut buf = [0u8; 64];
        let mut writer = VmWriter::from(buf.as_mut_bytes()).to_fallible();
        let mut printer = VmPrinter::new_skip(&mut writer, 100);

        let res = writeln!(printer, "short message");
        assert!(res.is_ok());

        // Nothing should be written
        assert_eq!(printer.bytes_written(), 0);
        assert_eq!(buf[0], 0);
    }
}
