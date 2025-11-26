// SPDX-License-Identifier: MPL-2.0

//! Aster-nix is the Asterinas kernel, a safe, efficient unix-like
//! operating system kernel built on top of OSTD and OSDK.

#![no_std]
#![no_main]
#![deny(unsafe_code)]
#![feature(btree_cursors)]
#![feature(btree_extract_if)]
#![feature(debug_closure_helpers)]
#![feature(extract_if)]
#![feature(format_args_nl)]
#![feature(integer_sign_cast)]
#![feature(let_chains)]
#![feature(linked_list_cursors)]
#![feature(linked_list_retain)]
#![feature(negative_impls)]
#![feature(panic_can_unwind)]
#![feature(register_tool)]
#![feature(min_specialization)]
#![feature(trait_alias)]
#![feature(trait_upcasting)]
#![feature(associated_type_defaults)]
#![register_tool(component_access_control)]

extern crate alloc;
extern crate lru;
#[macro_use]
extern crate controlled;
#[macro_use]
extern crate getset;

// We should declare components that are not explicitly used by others here to
// ensure they are linked into the final binary. Otherwise,
// `inventory::submit`ted `init` functions in those components will be discarded
// by the linker.
//
// See <https://github.com/dtolnay/inventory/issues/50>.
extern crate aster_mlsdisk;
extern crate aster_uart;

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
mod kcmdline;
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
