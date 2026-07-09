// SPDX-License-Identifier: MPL-2.0

//! System call dispatch in the AArch64 architecture.
//!
//! AArch64 uses the Linux `asm-generic` unified system-call table, shared with
//! RISC-V and LoongArch.

#[path = "./generic.rs"]
mod generic;

generic::define_syscalls_with_generic_syscall_table! {
    // TODO: Add AArch64-specific syscalls here.
}
