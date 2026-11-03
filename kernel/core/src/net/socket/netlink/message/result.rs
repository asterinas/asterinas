// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(netlink)]

use crate::prelude::*;

/// A type that indicates we either parsed a valid value, or hit a recoverable condition and can
/// continue reading.
///
/// If a method returns `SkippedInvalid(E)`, it is the callee's responsibility to consume or skip the
/// unreadable bytes so that the caller who receives this enum can keep reading from the next valid
/// boundary.
pub(crate) enum ContinueRead<T, E = Error> {
    /// A value has been parsed successfully.
    Parsed(T),
    /// A value has been skipped because it is invalid.
    SkippedInvalid(E),
}

/// A type that indicates we either parsed a valid attribute, or hit a recoverable condition and can
/// continue reading.
///
/// This is similar to [`ContinueRead`], except that it has an addition variant,
/// `SkippedUnsupported`. The variant indicates that the attribute type is not supported, including
/// types that are either unrecognized or unknown.
pub(crate) enum ContinueAttrRead<T> {
    /// An attribute has been parsed successfully.
    Parsed(T),
    /// An attribute has been skipped because its type is unsupported.
    SkippedUnsupported,
    /// An attribute has been skipped because it is invalid.
    SkippedInvalid(Error),
}

impl<T> ContinueRead<T, Error> {
    /// Creates a [`SkippedInvalid`] variant with the given error information.
    ///
    /// [`SkippedInvalid`]: Self::SkippedInvalid
    pub(in netlink) fn skipped_invalid(errno: Errno, msg: &'static str) -> Self {
        Self::SkippedInvalid(Error::with_message(errno, msg))
    }
}

impl<T> ContinueAttrRead<T> {
    /// Creates a [`SkippedInvalid`] variant with the given error information.
    ///
    /// [`SkippedInvalid`]: Self::SkippedInvalid
    pub(in netlink) fn skipped_invalid(errno: Errno, msg: &'static str) -> Self {
        Self::SkippedInvalid(Error::with_message(errno, msg))
    }
}

impl<T, E> ContinueRead<T, E> {
    /// Maps the value in the [`Parsed`] variant with `f`.
    ///
    /// [`Parsed`]: Self::Parsed
    pub(in netlink) fn map<F, U>(self, f: F) -> ContinueRead<U, E>
    where
        F: FnOnce(T) -> U,
    {
        match self {
            Self::Parsed(val) => ContinueRead::Parsed(f(val)),
            Self::SkippedInvalid(err) => ContinueRead::SkippedInvalid(err),
        }
    }

    /// Maps the error in the [`SkippedInvalid`] variant with `f`.
    ///
    /// [`SkippedInvalid`]: Self::SkippedInvalid
    pub(in netlink) fn map_err<F, U>(self, f: F) -> ContinueRead<T, U>
    where
        F: FnOnce(E) -> U,
    {
        match self {
            Self::Parsed(val) => ContinueRead::Parsed(val),
            Self::SkippedInvalid(err) => ContinueRead::SkippedInvalid(f(err)),
        }
    }
}
