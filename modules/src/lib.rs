// SPDX-License-Identifier: MPL-2.0

//! Aster-nix is the Asterinas kernel, a safe, efficient unix-like
//! operating system kernel built on top of OSTD and OSDK.

#![no_std]
#![no_main]
#![deny(unsafe_code)]
#![feature(btree_cursors)]
#![feature(btree_extract_if)]
#![feature(debug_closure_helpers)]
#![feature(extend_one)]
#![feature(extract_if)]
#![feature(fn_traits)]
#![feature(format_args_nl)]
#![feature(int_roundings)]
#![feature(integer_sign_cast)]
#![feature(let_chains)]
#![feature(linked_list_cursors)]
#![feature(linked_list_remove)]
#![feature(linked_list_retain)]
#![feature(negative_impls)]
#![feature(panic_can_unwind)]
#![feature(register_tool)]
#![feature(step_trait)]
#![feature(trait_alias)]
#![feature(trait_upcasting)]
#![feature(associated_type_defaults)]
#![register_tool(component_access_control)]

pub use aster_nix;

#[ostd::main]
pub fn main() {
    aster_nix::main();
}