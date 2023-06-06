//！This crate defines the require procedural macros to implement capability for jinux.
//! When use this crate, typeflags and typeflags-util should also be added as dependency.
//!
//! The require macro are used to ensure that an object has the enough capability to call the function.
//! The **require** macro can accept constraint [SomeRightSet] > [SomeRight],
//! which means the SomeRightSet should **contain** the SomeRight.
//! The **require** macro can also accept constraint [SomeRightSet] > [AnotherRightSet],
//! which means the SomeRightSet should **include** the AnotherRightSet. In this case, AnotherRightSet should be a **generic parameter**.
//! i.e., AnotherRightSet should occur the the generic param list of the function.
//!
//! If there are multiple constraits, they can be seperated with `|`, which means all constraits should be satisfied.
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
//!
//! #[static_cap(R)]
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
#![feature(iter_advance_by)]
#![feature(let_chains)]
#![allow(dead_code)]

use require_item::RequireItem;
use static_cap::expand_static_cap;
use static_cap::CapType;
use syn::parse_macro_input;
use syn::ItemImpl;

use crate::require_attr::expand_require;
use crate::require_attr::RequireAttr;

mod require_attr;
mod require_item;
mod static_cap;

#[proc_macro_attribute]
pub fn require(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let require_item = parse_macro_input!(item as RequireItem);
    let require_attr = parse_macro_input!(attr as RequireAttr);
    expand_require(require_item, require_attr).into()
}

/// The static_cap macro is used to replace all appearance of a generic parameter to a wrapper version in a impl block.
/// i.e., it will change generic parameter R to TypeWrapper<R> in impl header, impl items and etc. (TypeWrapper is defined in typeflags_util)
/// Below is a simple example.
///
/// ```ignore
/// #[static_cap(R)]
/// impl <R: RightSet> Channel<R> {
///     fn new() -> Channel<R> {
///         let r = R::new();
///         todo!()
///     }
///
///     #[require(R > Read)]
///     fn read(&self) {}
/// }
/// ```
/// After macro expansion, it will looks like
/// ```ignore
/// impl <R> Channel<TypeWrapper<R>> where TypeWrapper<R>: RightSet {
///     fn new() -> Channel<TypeWrapper<R>> {
///         let r = TypeWrapper::<R>::new()
///         todo!()
///     }
///
///     #[require(TypeWrapper<R> > Read)]
///     fn read(&self) {}
/// }
/// ```
#[proc_macro_attribute]
pub fn static_cap(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let item_impl = parse_macro_input!(item as ItemImpl);
    let cap_type = parse_macro_input!(attr as CapType);
    expand_static_cap(item_impl, cap_type).into()
}
