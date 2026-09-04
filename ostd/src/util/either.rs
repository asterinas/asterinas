// SPDX-License-Identifier: MPL-2.0

/// A type containing either a [`Left`] value `L` or a [`Right`] value `R`.
///
/// [`Left`]: Self::Left
/// [`Right`]: Self::Right
///
/// `Either` is an enum for the simple idea "this value is one or the other":
/// it holds either an `L` or an `R`, but never both, and the caller gets to
/// decide what each variant means. It is like `Result`, except `Result`'s
/// `Err` has a built-in "something went wrong" meaning, while `Either`
/// carries no meaning of its own — it is purely about saying "either this,
/// or that".
///
/// The names `Left` and `Right` are inherited from the same type in Haskell
/// (where `Either a b = Left a | Right b`). Do not try to read any meaning
/// into them: they do not mean "left side/right side", nor "good/bad". They
/// are simply the names chosen for the two slots of the enum, and which slot
/// is used for what is entirely up to the user of the type.
///
/// For example, an XArray slot holds either an internal node or a user item.
/// The code that writes it decides: `Either::Left(node)` means "a node",
/// `Either::Right(item)` means "an item" — a choice that is obvious at the
/// call site, not from the enum name or the words "left"/"right".
///
/// Useful methods: [`left`](Self::left) / [`right`](Self::right) extract the
/// value if it is on that side, and [`is_left`](Self::is_left) /
/// [`is_right`](Self::is_right) check which side is present.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
