// SPDX-License-Identifier: MPL-2.0

//! Type level bools

pub use core::ops::{BitAnd as And, BitOr as Or, Not};
use core::unimplemented;

pub trait Bool {}

/// Type-level "true".
pub struct True;

/// Type-level "false".
pub struct False;

impl Bool for True {}
impl Bool for False {}

impl Not for True {
    type Output = False;

    fn not(self) -> Self::Output {
        unimplemented!("not supposed to be used")
    }
}

impl Not for False {
    type Output = True;

    fn not(self) -> Self::Output {
        unimplemented!("not supposed to be used")
    }
}

impl<B: Bool> And<B> for True {
    type Output = B;

    fn bitand(self, _rhs: B) -> Self::Output {
        unimplemented!("not supposed to be used")
    }
}

impl<B: Bool> And<B> for False {
    type Output = False;

    fn bitand(self, _rhs: B) -> Self::Output {
        unimplemented!("not supposed to be used")
    }
}

impl<B: Bool> Or<B> for True {
    type Output = True;

    fn bitor(self, _rhs: B) -> Self::Output {
        unimplemented!("not supposed to be used")
    }
}

impl<B: Bool> Or<B> for False {
    type Output = B;

    fn bitor(self, _rhs: B) -> Self::Output {
        unimplemented!("not supposed to be used")
    }
}

pub type NotOp<B> = <B as Not>::Output;
pub type AndOp<B0, B1> = <B0 as And<B1>>::Output;
pub type OrOp<B0, B1> = <B0 as Or<B1>>::Output;

// In where clause, we can only use trait bounds, but not equal bounds.
// For certain situation, we need to do some comparison in where clause.
// e.g., we need to determine the result type of `SetContainOp` is `True` or `False`.
// Since Sometype == True is not allowed, We can use SomeType: IsTrue to determine the result.
pub trait IsTrue {}
pub trait IsFalse {}

impl IsTrue for True {}
impl IsFalse for False {}
