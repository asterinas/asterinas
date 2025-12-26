// SPDX-License-Identifier: MPL-2.0

//! Aster-nix is the Asterinas kernel, a safe, efficient unix-like
//! operating system kernel built on top of OSTD and OSDK.

#![no_std]
#![no_main]
#![deny(unsafe_code)]
#![feature(btree_cursors)]
#![feature(debug_closure_helpers)]
#![feature(format_args_nl)]
#![feature(linked_list_cursors)]
#![feature(linked_list_retain)]
#![feature(negative_impls)]
#![feature(panic_can_unwind)]
#![feature(register_tool)]
#![feature(min_specialization)]
#![feature(thin_box)]
#![feature(trait_alias)]
#![feature(associated_type_defaults)]
#![register_tool(component_access_control)]

extern crate alloc;
extern crate lru;
#[macro_use]
extern crate controlled;
#[macro_use]
extern crate getset;

#[cfg_attr(target_arch = "x86_64", path = "arch/x86/mod.rs")]
#[cfg_attr(target_arch = "riscv64", path = "arch/riscv/mod.rs")]
#[cfg_attr(target_arch = "loongarch64", path = "arch/loongarch/mod.rs")]
mod arch;

mod context;
mod cpu;
mod device;
mod driver;
mod error;
mod events;
mod fs;
mod init;
mod ipc;
mod net;
mod prelude;
mod process;
mod sched;
mod security;
mod syscall;
mod thread;
mod time;
mod util;
// TODO: Add vDSO support for other architectures.
#[cfg(any(target_arch = "x86_64", target_arch = "riscv64"))]
mod vdso;
mod vm;

#[ostd::main]
#[controlled]
fn main() {
    init::main();
}
