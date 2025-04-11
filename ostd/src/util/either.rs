// SPDX-License-Identifier: MPL-2.0

/// A type containing either a [`Left`] value `L` or a [`Right`] value `R`.
///
/// [`Left`]: Self::Left
/// [`Right`]: Self::Right
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Either<L, R> {
    /// Contains the left value
    Left(L),
    /// Contains the right value
    Right(R),
}

impl<L, R> Either<L, R> {
    /// Converts to the left value, if any.
    pub fn left(self) -> Option<L> {
        match self {
            Self::Left(left) => Some(left),
            Self::Right(_) => None,
        }
    }

    /// Converts to the right value, if any.
    pub fn right(self) -> Option<R> {
        match self {
            Self::Left(_) => None,
            Self::Right(right) => Some(right),
        }
    }

    /// Returns true if the left value is present.
    pub fn is_left(&self) -> bool {
        matches!(self, Self::Left(_))
    }

    /// Returns true if the right value is present.
    pub fn is_right(&self) -> bool {
        matches!(self, Self::Right(_))
    }

    // TODO: Add other utility methods (e.g. `as_ref`, `as_mut`) as needed.
    // As a good reference, check what methods `Result` provides.
}
