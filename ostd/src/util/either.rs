// SPDX-License-Identifier: MPL-2.0

/// A type containing either a value of type `L` or a value of type `R`.
pub enum Either<L, R> {
    Left(L),
    Right(R),
}

impl<L, R> Either<L, R> {
    /// Converts the left side of `Either<L, R>` to an `Option<L>`.
    pub fn left(self) -> Option<L> {
        match self {
            Self::Left(l) => Some(l),
            Self::Right(_) => None,
        }
    }

    /// Converts the right side of `Either<L, R>` to an `Option<R>`.
    pub fn right(self) -> Option<R> {
        match self {
            Self::Left(_) => None,
            Self::Right(r) => Some(r),
        }
    }

    /// Returns true if the value is the `Left` variant.
    pub fn is_left(&self) -> bool {
        match self {
            Self::Left(_) => true,
            Self::Right(_) => false,
        }
    }

    /// Returns true if the value is the `Right` variant.
    pub fn is_right(&self) -> bool {
        !self.is_left()
    }
}
