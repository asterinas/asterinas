// SPDX-License-Identifier: MPL-2.0

//! System call dispatch in the LoongArch architecture.

#[path = "./generic.rs"]
mod generic;

generic::define_syscalls_with_generic_syscall_table! {
    // TODO: Add LoongArch specific syscalls here.
}
