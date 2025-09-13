// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

/// A type that indicates we either parsed a valid value, or hit a recoverable condition and can
/// continue reading.
///
/// If a method returns `Skipped` or `SkippedErr(E)`, it is the callee's responsibility to consume
/// or skip the unreadable bytes so that the caller who receives this enum can keep reading from
/// the next valid boundary.
pub enum ContinueRead<T, E = Error> {
    Parsed(T),
    Skipped,
    SkippedErr(E),
}

impl<T> ContinueRead<T, Error> {
    /// Creates a [`SkippedErr`] variant with the given error information.
    ///
    /// [`SkippedErr`]: Self::SkippedErr
    pub fn skipped_with_error(errno: Errno, msg: &'static str) -> Self {
        Self::SkippedErr(Error::with_message(errno, msg))
    }
}

impl<T, E> ContinueRead<T, E> {
    /// Maps the value in the [`Parsed`] variant with `f`.
    ///
    /// [`Parsed`]: Self::Parsed
    pub fn map<F, U>(self, f: F) -> ContinueRead<U, E>
    where
        F: FnOnce(T) -> U,
    {
        match self {
            Self::Parsed(val) => ContinueRead::Parsed(f(val)),
            Self::Skipped => ContinueRead::Skipped,
            Self::SkippedErr(err) => ContinueRead::SkippedErr(err),
        }
    }

    /// Maps the error in the [`SkippedErr`] variant with `f`.
    ///
    /// [`SkippedErr`]: Self::SkippedErr
    pub fn map_err<F, U>(self, f: F) -> ContinueRead<T, U>
    where
        F: FnOnce(E) -> U,
    {
        match self {
            Self::Parsed(val) => ContinueRead::Parsed(val),
            Self::Skipped => ContinueRead::Skipped,
            Self::SkippedErr(err) => ContinueRead::SkippedErr(f(err)),
        }
    }
}
