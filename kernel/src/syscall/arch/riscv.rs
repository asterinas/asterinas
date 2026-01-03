// SPDX-License-Identifier: MPL-2.0

//! System call dispatch in the RISC-V architecture.

#[path = "./generic.rs"]
mod generic;

generic::define_syscalls_with_generic_syscall_table! {
    // TODO: Add RISC-V specific syscalls here.
}
