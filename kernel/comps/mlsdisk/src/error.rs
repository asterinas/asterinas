// SPDX-License-Identifier: MPL-2.0

use core::fmt;

/// The error types used in this crate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Errno {
    /// Transaction aborted.
    TxAborted,
    /// Not found.
    NotFound,
    /// Invalid arguments.
    InvalidArgs,
    /// Out of memory.
    OutOfMemory,
    /// Out of disk space.
    OutOfDisk,
    /// IO error.
    IoFailed,
    /// Permission denied.
    PermissionDenied,
    /// Unsupported.
    Unsupported,
    /// OS-specific unknown error.
    OsSpecUnknown,
    /// Encryption operation failed.
    EncryptFailed,
    /// Decryption operation failed.
    DecryptFailed,
    /// MAC (Message Authentication Code) mismatched.
    MacMismatched,
    /// Not aligned to `BLOCK_SIZE`.
    NotBlockSizeAligned,
    /// Try lock failed.
    TryLockFailed,
}

/// The error with an error type and an error message used in this crate.
#[derive(Clone, Debug)]
pub struct Error {
    errno: Errno,
    msg: Option<&'static str>,
}

impl Error {
    /// Creates a new error with the given error type and no error message.
    pub const fn new(errno: Errno) -> Self {
        Error { errno, msg: None }
    }

    /// Creates a new error with the given error type and the error message.
    pub const fn with_msg(errno: Errno, msg: &'static str) -> Self {
        Error {
            errno,
            msg: Some(msg),
        }
    }

    /// Returns the error type.
    pub fn errno(&self) -> Errno {
        self.errno
    }
}

impl From<Errno> for Error {
    fn from(errno: Errno) -> Self {
        Error::new(errno)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl fmt::Display for Errno {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[macro_export]
macro_rules! return_errno {
    ($errno: expr) => {
        return core::result::Result::Err($crate::Error::new($errno))
    };
}

#[macro_export]
macro_rules! return_errno_with_msg {
    ($errno: expr, $msg: expr) => {
        return core::result::Result::Err($crate::Error::with_msg($errno, $msg))
    };
}
