// SPDX-License-Identifier: MPL-2.0

pub type Result<T> = core::result::Result<T, self::Error>;

/// Errors of CPIO decoder.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    MagicError,
    Utf8Error,
    ParseIntError,
    FileTypeError,
    FileNameError,
    BufferShortError,
    IoError,
}

impl From<core2::io::Error> for Error {
    #[inline]
    fn from(err: core2::io::Error) -> Self {
        use core2::io::ErrorKind;

        match err.kind() {
            ErrorKind::UnexpectedEof => Self::BufferShortError,
            _ => Self::IoError,
        }
    }
}
