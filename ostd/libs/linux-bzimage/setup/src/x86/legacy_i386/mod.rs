// SPDX-License-Identifier: MPL-2.0

use core::arch::{asm, global_asm};

global_asm!(include_str!("setup.S"));

use crate::console::{print_hex, print_str};

pub const ASTER_ENTRY_POINT: u32 = 0x8001000;

#[export_name = "main_legacy32"]
extern "cdecl" fn main_legacy32(boot_params_ptr: u32) -> ! {
    // SAFETY: this init function is only called once.
    unsafe { crate::console::init() };

    // println!("[setup] bzImage loaded at {:#x}", x86::relocation::image_load_offset());
    unsafe {
        print_str("[setup] bzImage loaded offset: ");
        print_hex(crate::x86::image_load_offset() as u64);
        print_str("\n");
    }

    crate::loader::load_elf(crate::x86::payload());

    // SAFETY: the entrypoint and the ptr is valid.
    unsafe { call_aster_entrypoint(ASTER_ENTRY_POINT, boot_params_ptr.try_into().unwrap()) };
}

unsafe fn call_aster_entrypoint(entrypoint: u32, boot_params_ptr: u32) -> ! {
    asm!("mov esi, {}", in(reg) boot_params_ptr);
    asm!("mov eax, {}", in(reg) entrypoint);
    asm!("jmp eax");

    unreachable!();
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
