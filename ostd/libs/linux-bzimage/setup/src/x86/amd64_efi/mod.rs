// SPDX-License-Identifier: MPL-2.0

pub(super) mod alloc;
mod decoder;
mod efi;

use core::arch::{asm, global_asm};

use linux_boot_params::BootParams;

global_asm!(include_str!("setup.S"));

const ASTER_ENTRY_POINT: *const () = 0x8001200 as _;

unsafe fn call_aster_entrypoint(entrypoint: *const (), boot_params_ptr: *mut BootParams) -> ! {
    unsafe {
        asm!(
            "mov rsi, {1}",
            "mov rax, {0}",
            "jmp rax",
            in(reg) entrypoint,
            in(reg) boot_params_ptr,
            options(noreturn),
        )
    }
}
