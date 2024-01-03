// SPDX-License-Identifier: MPL-2.0

mod efi;
mod paging;
mod relocation;

use core::arch::{asm, global_asm};

global_asm!(include_str!("header.S"));

global_asm!(include_str!("setup.S"));

pub const ASTER_ENTRY_POINT: u32 = 0x8001200;

unsafe fn call_aster_entrypoint(entrypoint: u64, boot_params_ptr: u64) -> ! {
    asm!("mov rsi, {}", in(reg) boot_params_ptr as u64);
    asm!("mov rax, {}", in(reg) entrypoint as u64);
    asm!("jmp rax");

    unreachable!();
}
