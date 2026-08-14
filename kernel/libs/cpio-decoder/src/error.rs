// SPDX-License-Identifier: MPL-2.0

pub type Result<T, E = Error> = core::result::Result<T, E>;

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

impl From<alloc::io::Error> for Error {
    fn from(err: alloc::io::Error) -> Self {
        use alloc::io::ErrorKind;

        match err.kind() {
            ErrorKind::UnexpectedEof => Self::BufferShortError,
            _ => Self::IoError,
        }
    }
}
