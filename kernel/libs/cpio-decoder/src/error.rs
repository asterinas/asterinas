// SPDX-License-Identifier: MPL-2.0

pub type Result<T> = core::result::Result<T, self::Error>;

/// Errors of CPIO decoder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    MagicError,
    Utf8Error,
    ParseIntError,
    FileTypeError,
    FileNameError,
    BufferShortError,
    IoError,
}

impl From<no_std_io2::io::Error> for Error {
    #[inline]
    fn from(err: no_std_io2::io::Error) -> Self {
        use no_std_io2::io::ErrorKind;

        match err.kind() {
            ErrorKind::UnexpectedEof => Self::BufferShortError,
            _ => Self::IoError,
        }
    }
}
