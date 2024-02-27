// SPDX-License-Identifier: MPL-2.0

//! Traits used to check if two types are the same, returning a Bool.
//! This check happens at compile time

use crate::bool::{Bool, False, True};

pub trait SameAs<T> {
    type Output: Bool;
}

// A type is always same as itself
impl<T> SameAs<T> for T {
    type Output = True;
}

impl SameAs<False> for True {
    type Output = False;
}

impl SameAs<True> for False {
    type Output = False;
}

// How to implement self reflection?
// impl <T, U> SameAs<T> for U where T: SameAs<U>, {
//     type Output = <U as SameAs<T>>::Output;
// }

pub type SameAsOp<T, U> = <T as SameAs<U>>::Output;
