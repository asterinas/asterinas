// SPDX-License-Identifier: MPL-2.0

//！This crate defines the procedural macro typeflags to implement capability for Asterinas.
//! When using this crate, typeflags-util should also be added as dependency.
//! This is due to typeflgas is a proc-macro crate, which is only allowed to export proc-macro interfaces.
//! So we leave the common type-level operations and structures defined in typeflags-util.
//!
//! typeflags is used to define another declarive macro to define type set.
//! It can be used as the following example.
//! ```rust
//! use typeflags::typeflags;
//! typeflags! {
//!     pub trait RightSet: u32 {
//!          struct Read = 1 << 1;
//!          struct Write = 1 << 2;
//!     }
//! }
//!
//! // The above code will generate a macro with the name as RightSet, we can use this macro to define typesets with different types.
//! // Usage example:
//! type O = RightSet![];               // Nil
//! type R = RightSet![Read];           // Cons<Read, Nil>
//! type W = RightSet![Write];          // Cons<Write, Nil>
//! type RW = RightSet![Read, Write];   // Cons<Write, Cons<Read, Nil>>
//! type WR = RightSet![Write, Read];   // Cons<Write, Cons<Read, Nil>>
//!
//! // Test Example
//! extern crate typeflags_util;
//! use typeflags_util::*;
//! assert_eq!(O::BITS, 0);
//! assert_eq!(R::BITS, 2);
//! assert_eq!(W::BITS, 4);
//! assert_eq!(RW::BITS, 6);
//! assert_eq!(WR::BITS, 6);
//! assert_type_same!(SameAsOp<Read, Write>, False);
//! assert_type_same!(SameAsOp<Write, Write>, True);
//! assert_type_same!(SameAsOp<O, Nil>, True);
//! assert_type_same!(SameAsOp<RW, WR>, True);
//! assert_type_same!(SetContainOp<R, Write>, False);
//! assert_type_same!(SetContainOp<RW, Read>, True);
//! assert_type_same!(SetContainOp<O, Read>, False);
//! assert_type_same!(SetContainOp<R, Read>, True);
//! assert_type_same!(SetIncludeOp<RW, R>, True);
//! assert_type_same!(SetIncludeOp<R, W>, False);
//! assert_type_same!(SetIncludeOp<W, O>, True);
//! assert_type_same!(SetIncludeOp<O, R>, False);
//! assert_type_same!(SetExtendOp<O, Read>, R);
//! assert_type_same!(SetExtendOp<R, Write>, RW);
//! assert_type_same!(SetExtendOp<R, Read>, R);
//! assert_type_same!(SetExtendOp<W, Read>, RW);
//! ```

#![feature(proc_macro_diagnostic)]
#![allow(dead_code)]

use syn::parse_macro_input;

use crate::{type_flag::TypeFlagDef, util::expand_type_flag};

mod flag_set;
mod type_flag;
mod util;

#[proc_macro]
pub fn typeflags(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let type_flags_def = parse_macro_input!(input as TypeFlagDef);
    expand_type_flag(&type_flags_def).into()
}
