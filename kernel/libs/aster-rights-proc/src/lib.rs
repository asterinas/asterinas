// SPDX-License-Identifier: MPL-2.0

//！This crate defines the require procedural macros to implement capability for Asterinas.
//! When use this crate, typeflags and typeflags-util should also be added as dependency.
//!
//! The require macro are used to ensure that an object has the enough capability to call the function.
//! The **require** macro can accept constraint `SomeRightSet` > `SomeRight`,
//! which means the `SomeRightSet` should **contain** the `SomeRight`.
//! The **require** macro can also accept constraint `SomeRightSet` > `AnotherRightSet`,
//! which means the `SomeRightSet` should **include** the `AnotherRightSet`. In this case, `AnotherRightSet` should be a **generic parameter**.
//! i.e., `AnotherRightSet` should occur the the generic param list of the function.
//!
//! If there are multiple constraints, they can be separated with `|`, which means all constraints should be satisfied.
//!
//! The require can also be used multiple times, which means each macro should be satisfied.
//!
//! Below is a simple example.
//! Suppose we have a channel that may have read and write capability.
//!
//! ```ignore
//! /// A simple channel, only for demonstration.
//! struct Channel<R: RightSet> {
//!    rights: PhantomData<R>,
//! }
//! impl <R: RightSet> Channel<R> {
//!     pub fn new() -> Self {
//!         Channel { rights: PhantomData }
//!     }

//!     #[require(R > Read)]
//!     pub fn read(&self) {}
//!
//!     #[require(R > Write)]
//!     pub fn write(&self) {}
//!
//!     #[require(R > Read | Write)]
//!     pub fn read_write(&self) {}     

//!     #[require(R > R1)]
//!     pub fn restrict<R1>(self) -> Channel<R1> where R1: RightSet {
//!         Channel::new()           
//!     }
//! }    
//! ```
//! When we initialize channels with different rights, it can check whether the function
//! are wrongly used due to lack of capabilities at compilation time.
//! ```ignore
//!     let read_channel = Channel::<R>::new();
//!     read_channel.read();
//!     // read_channel.write();                    // compilation error!
//!     // read_channel.read_write();               // compilation error!
//!     let _ = read_channel.restrict::<R>();
//!
//!     let write_channel = Channel::<W>::new();
//!     write_channel.write();
//!     // write_channel.read();                    // compilation error!
//!     // write_channel.read_write();              // compilation error!
//!     let _ = write_channel.restrict::<O>();
//!     // let _ = write_channel.restrict::<R>();   // compilation error!
//!
//!     let rw_channel = Channel::<RW>::new();
//!     rw_channel.read();
//!     rw_channel.write();
//!     rw_channel.read_write();
//!     let rchannel = rw_channel.restrict::<R>();
//!     rchannel.read();
//!     // rchannel.write();                        // compilation error!
//!
//!     let no_rw_channel = Channel::<O>::new();
//!     // no_rw_channel.read();                    // compilation error!
//!     // no_rw_channel.write();                   // compilation error!
//!     // no_rw_channel.read_write();              // compilation error!
//! ```

#![feature(proc_macro_diagnostic)]
#![allow(dead_code)]

use require_item::RequireItem;
use syn::parse_macro_input;

use crate::require_attr::{expand_require, RequireAttr};

mod require_attr;
mod require_item;

#[proc_macro_attribute]
pub fn require(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let require_item = parse_macro_input!(item as RequireItem);
    let require_attr = parse_macro_input!(attr as RequireAttr);
    expand_require(require_item, require_attr).into()
}
