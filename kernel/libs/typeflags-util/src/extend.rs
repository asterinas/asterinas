// SPDX-License-Identifier: MPL-2.0

use crate::{Cons, Nil};

/// This trait will extend a set with another item.
///
/// If the set already contains the item, it will return the original set.
/// Otherwise, it will return the new set with the new item.
/// The implementation should care about the item orders when extending set.
pub trait SetExtend<T> {
    type Output;
}

pub type SetExtendOp<Set, T> = <Set as SetExtend<T>>::Output;

impl<T> SetExtend<T> for Nil {
    type Output = Cons<T, Nil>;
}
