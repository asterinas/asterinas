// SPDX-License-Identifier: MPL-2.0

//! The content of this crate is from another project CapComp.
//! This crate defines common type level operations, like SameAsOp, and Bool type operations.
//! Besides, this crate defines operations to deal with type sets, like SetContain and SetInclude.
//! When use typeflags or aster-rights-poc, this crate should also be added as a dependency.
#![no_std]
pub mod assert;
pub mod bool;
pub mod extend;
pub mod if_;
pub mod same;
pub mod set;

pub use assert::AssertTypeSame;

pub use crate::{
    bool::{And, AndOp, False, IsFalse, IsTrue, Not, NotOp, Or, OrOp, True},
    extend::{SetExtend, SetExtendOp},
    same::{SameAs, SameAsOp},
    set::{Cons, Nil, Set, SetContain, SetContainOp, SetInclude, SetIncludeOp},
};
