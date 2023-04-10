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
}
