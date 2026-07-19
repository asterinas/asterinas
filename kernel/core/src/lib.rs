// SPDX-License-Identifier: MPL-2.0

//! The core of the Asterinas kernel.
//!
//! This crate implements the Linux ABI and the core kernel mechanisms.

#![no_std]
#![deny(unsafe_code)]
#![feature(array_try_from_fn)]
#![feature(associated_type_defaults)]
#![feature(btree_cursors)]
#![feature(debug_closure_helpers)]
#![feature(format_args_nl)]
#![feature(linked_list_cursors)]
#![feature(linked_list_retain)]
#![feature(panic_can_unwind)]
#![feature(min_specialization)]
#![feature(thin_box)]
#![feature(unique_rc_arc)]
#![feature(vec_deque_truncate_front)]

extern crate alloc;
extern crate lru;
#[macro_use]
extern crate getset;
#[macro_use]
extern crate ostd_pod;

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        ""
    };
}

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

/// Boots the Asterinas core kernel.
pub fn boot() {
    init::main();
}
